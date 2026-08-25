//! Tauri commands for talking to the Inkwash device over USB or BLE.
//!
//! The two phases of every command are split so the application-wide
//! `state.link` mutex is *released* before the slow `recv_timeout()`
//! happens:
//!
//! 1. Lock the link just long enough to push the command onto the
//!    worker's `mpsc::Sender`. Drop the guard immediately.
//! 2. Lock the per-link `event_rx` mutex, wait up to the deadline, drop.
//!    Other commands (BLE scans, log reads, server CRUD) can run while
//!    we're waiting.
//!
//! The transport-level workers run on plain OS threads with blocking
//! `mpsc` channels, so the slow `recv_timeout` here is a regular
//! blocking call wrapped in `tauri::async_runtime::spawn_blocking` at
//! each command entry point - the UI thread is never blocked.
//!
//! See `state::AppState` for the design rationale and `protocol` for
//! the wire format.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::desktop::SharedState;
use crate::error::AppError;
use crate::protocol::{self, Command, Reply};
use crate::state::{ActiveLink, AppState, InflightRegistration, LinkKind, LinkState};
use crate::transport::{ble::BleLink, usb::UsbLink, Event, Transport};

const DEVICE_CMD_TIMEOUT: Duration = Duration::from_secs(45);
// Opening the ESP32-S3 USB Serial/JTAG port resets the chip, so a command
// sent immediately after connect can be lost while the device boots. Resend
// on USB until a reply arrives - same behaviour as the CLI (`main.rs`).
// All protocol commands are idempotent, so a resent command is safe.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCommandResult {
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<DeviceStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub wifi_configured: bool,
    pub server_configured: bool,
    pub wifi_connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wifi_ssid: Option<String>,
    pub wifi_has_password: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    pub server_has_token: bool,
    pub timezone_offset_minutes: i16,
}

fn result_from_reply(reply: Reply) -> DeviceCommandResult {
    match reply {
        Reply::Ok => DeviceCommandResult {
            kind: "ok".into(),
            message: "Device accepted the command".into(),
            status: None,
        },
        Reply::Status {
            wifi_configured,
            server_configured,
            wifi_connected,
            wifi_ssid,
            wifi_has_password,
            server_url,
            server_has_token,
            timezone_offset_minutes,
        } => DeviceCommandResult {
            kind: "status".into(),
            message: "Device status received".into(),
            status: Some(DeviceStatus {
                wifi_configured,
                server_configured,
                wifi_connected,
                wifi_ssid,
                wifi_has_password,
                server_url,
                server_has_token,
                timezone_offset_minutes,
            }),
        },
        Reply::Error { message } => DeviceCommandResult {
            kind: "error".into(),
            message,
            status: None,
        },
        // `drive_request` intercepts `Busy` itself (auto-retry) before
        // this function ever sees one; this arm only exists for
        // exhaustiveness.
        Reply::Busy => DeviceCommandResult {
            kind: "busy".into(),
            message: "Device is showing a reminder; retrying".into(),
            status: None,
        },
    }
}

/// Snapshot of the connection a request was started on: the transport
/// itself (an owned `Arc`, so the wait loop can poll it without holding
/// the `link` mutex) plus which radio it runs on. A stale snapshot (link
/// swapped out from under us) is detected by pointer identity and simply
/// means "not connected" for this request.
struct Phase {
    transport: Arc<dyn Transport>,
    kind: LinkKind,
}

/// How often the retry driver wakes up between events while waiting.
const RETRY_TICK: Duration = Duration::from_millis(250);

/// Side-channel notices emitted while driving a request, so the GUI's
/// log store and the CLI's stdout can render the same retry decisions
/// their own way.
pub(crate) enum RetryNotice {
    /// Device log noise surfaced mid-wait.
    DeviceLog(String),
    /// A non-stale reply arrived (already rendered as JSON).
    IncomingReply(String),
    /// A reply for a different request id was ignored (`"?"` when the
    /// device sent no id at all).
    StaleReply(String),
    /// The device answered `busy`; the command is being resent.
    Busy,
    /// A resend attempt couldn't be queued on the link.
    ResendFailed(String),
}

/// Why a driven request ended without a final reply.
#[derive(Debug)]
pub(crate) enum RetryError {
    TimedOut,
    /// The transport reported a disconnect.
    Disconnected(String),
    /// The link was swapped out or cleared underneath this request.
    LinkLost,
    /// The very first copy couldn't even be queued (worker gone).
    SendFailed(String),
}

/// One receive tick against the active link.
pub(crate) enum Tick {
    Event(Event),
    /// Nothing arrived within the tick window.
    Idle,
    /// The request can no longer make progress on this link.
    Lost,
}

/// How [`drive_request`] reaches whichever link is active. Implemented
/// twice: over the GUI's shared `AppState` link (with stale-snapshot
/// checks under the mutex) and over the CLI's bare `UsbLink`.
pub(crate) trait RetryLink {
    /// Queues one copy of `command` under `request_id`.
    fn send(&mut self, request_id: &str, command: &Command) -> Result<(), String>;

    /// Waits up to `tick` for the next event from the transport.
    fn recv(&mut self, tick: Duration) -> Tick;
}

fn resend_now(
    link: &mut dyn RetryLink,
    request_id: &str,
    command: &Command,
    retry_interval: Duration,
    next_resend: &mut Instant,
    notify: &mut dyn FnMut(RetryNotice),
) {
    if let Err(err) = link.send(request_id, command) {
        notify(RetryNotice::ResendFailed(err));
    }
    *next_resend = Instant::now() + retry_interval;
}

/// Resend/timeout core of every device request, shared by the Tauri
/// command path ([`send_and_wait`]) and the headless CLI
/// (`main.rs::cli_usb_command`): drive `command` under one correlation
/// id until a matching non-busy reply arrives or `deadline` expires,
/// resending every `retry_interval` of silence. The resends exist
/// because opening the ESP32-S3 USB Serial/JTAG port resets the chip, so
/// early copies of a command can be lost while it boots; all protocol
/// commands are idempotent, so a resent command is safe.
///
/// Reply matching follows `protocol::classify_reply`: a reply carrying a
/// *different* id belongs to some other in-flight/stale request and is
/// surfaced as [`RetryNotice::StaleReply`] then ignored; a reply with no
/// id at all is accepted on trust (firmware predating correlation); a
/// `busy` reply triggers an immediate resend of the same request.
///
/// `resend_while_idle` controls silence-triggered resends: true over USB
/// (boot-reset protection), false over BLE, whose link only exists after
/// GATT setup completed and whose commands are never lost to a boot.
pub(crate) fn drive_request(
    link: &mut dyn RetryLink,
    request_id: &str,
    command: &Command,
    deadline: Instant,
    retry_interval: Duration,
    resend_while_idle: bool,
    notify: &mut dyn FnMut(RetryNotice),
) -> Result<Reply, RetryError> {
    if let Err(err) = link.send(request_id, command) {
        return Err(RetryError::SendFailed(err));
    }
    let mut next_resend = Instant::now() + retry_interval;
    loop {
        if Instant::now() >= deadline {
            return Err(RetryError::TimedOut);
        }
        match link.recv(RETRY_TICK) {
            Tick::Lost => return Err(RetryError::LinkLost),
            Tick::Idle => {
                if resend_while_idle && Instant::now() >= next_resend {
                    resend_now(link, request_id, command, retry_interval, &mut next_resend, notify);
                }
            }
            Tick::Event(Event::Log(line)) => notify(RetryNotice::DeviceLog(line)),
            Tick::Event(Event::Disconnected(reason)) => {
                return Err(RetryError::Disconnected(reason));
            }
            Tick::Event(Event::Reply(reply_id, reply)) => {
                match protocol::classify_reply(request_id, reply_id.as_deref(), &reply) {
                    protocol::ReplyDecision::Stale => {
                        notify(RetryNotice::StaleReply(
                            reply_id.as_deref().unwrap_or("?").to_string(),
                        ));
                    }
                    decision => {
                        notify(RetryNotice::IncomingReply(
                            serde_json::to_string(&reply)
                                .unwrap_or_else(|_| "<unprintable>".into()),
                        ));
                        if decision == protocol::ReplyDecision::Busy {
                            notify(RetryNotice::Busy);
                            resend_now(
                                link,
                                request_id,
                                command,
                                retry_interval,
                                &mut next_resend,
                                notify,
                            );
                            continue;
                        }
                        return Ok(reply);
                    }
                }
            }
        }
    }
}

/// [`RetryLink`] over the application-wide shared state: every access
/// re-checks under the `link` mutex that this request's transport is
/// still the active one, mirroring the stale-phase handling.
struct AppStateLink<'a> {
    state: &'a AppState,
    phase: &'a Phase,
}

impl RetryLink for AppStateLink<'_> {
    fn send(&mut self, request_id: &str, command: &Command) -> Result<(), String> {
        let guard = self
            .state
            .link
            .lock()
            .map_err(|e| format!("link mutex poisoned: {e}"))?;
        match &*guard {
            LinkState::Connected(link)
                if Arc::ptr_eq(&link.transport(), &self.phase.transport) =>
            {
                link.transport()
                    .send(request_id, command.clone())
                    .map_err(|e| e.to_string())
            }
            _ => Err("device link is no longer active".into()),
        }
    }

    fn recv(&mut self, tick: Duration) -> Tick {
        let received = {
            let Ok(guard) = self.state.link.lock() else {
                return Tick::Lost;
            };
            match &*guard {
                LinkState::Connected(link)
                    if Arc::ptr_eq(&link.transport(), &self.phase.transport) =>
                {
                    // The snapshot Arc keeps receiving events after this
                    // lock guard drops - see state.rs invariant 1.
                    link.transport().recv_timeout(tick)
                }
                _ => return Tick::Lost,
            }
        };
        match received {
            Ok(event) => Tick::Event(event),
            Err(mpsc::RecvTimeoutError::Timeout) => Tick::Idle,
            Err(mpsc::RecvTimeoutError::Disconnected) => Tick::Lost,
        }
    }
}

/// Send `command` to the device and wait for its `Reply`. Synchronous -
/// blocks for up to `DEVICE_CMD_TIMEOUT`. Wrap in `spawn_blocking` at
/// the command entry point.
///
/// Identical concurrent commands are coalesced (T-050): while one
/// request is in flight, a second identical one - e.g. the Overview and
/// Device pages both firing `get_status` right after connect_usb, during
/// the ~25s USB boot window - shares the first request's result instead
/// of putting a duplicate copy on the wire. The coalescing decision is
/// logged so it can be verified on hardware.
fn send_and_wait(state: &AppState, command: Command) -> Result<DeviceCommandResult, AppError> {
    let key = serde_json::to_string(&command).unwrap_or_else(|_| format!("{command:?}"));
    match state.inflight.register(&key) {
        InflightRegistration::Joined(waiter) => {
            state.logs.info(
                "device",
                format!(
                    "duplicate startup probe coalesced: identical {key} already in flight; sharing its result"
                ),
            );
            waiter.wait(Instant::now() + DEVICE_CMD_TIMEOUT)
        }
        InflightRegistration::Leader(guard) => {
            let result = drive_device_command(state, command);
            guard.finish(result.clone());
            result
        }
    }
}

/// The leader path: actually queue `command` on the active transport and
/// drive it to completion via the shared retry driver.
fn drive_device_command(
    state: &AppState,
    command: Command,
) -> Result<DeviceCommandResult, AppError> {
    let request_id = protocol::next_request_id();

    // Log first - the command is queued by `drive_request` below and may
    // be resent while the device boots, so log before the first send.
    state.logs.info(
        "device",
        format!(
            "→ [{request_id}] {}",
            serde_json::to_string(&command).unwrap_or_else(|_| "<unprintable>".into())
        ),
    );

    let phase = {
        let guard = state
            .link
            .lock()
            .map_err(|e| AppError::internal(format!("link mutex poisoned: {e}")))?;
        match &*guard {
            LinkState::Disconnected => return Err(AppError::device_not_connected()),
            LinkState::Connected(link) => Phase {
                transport: link.transport(),
                kind: link.kind(),
            },
        }
    };

    let mut link = AppStateLink { state, phase: &phase };
    let reply = drive_request(
        &mut link,
        &request_id,
        &command,
        Instant::now() + DEVICE_CMD_TIMEOUT,
        RETRY_INTERVAL,
        // Silence-triggered resends are a USB boot-reset workaround only.
        phase.kind == LinkKind::Usb,
        &mut |notice| match notice {
            RetryNotice::DeviceLog(line) => state.logs.info("device-log", line),
            RetryNotice::IncomingReply(json) => state.logs.info("device", format!("← {json}")),
            RetryNotice::StaleReply(id) => state.logs.info(
                "device",
                format!(
                    "← reply id '{id}' does not match in-flight request '{request_id}'; ignoring"
                ),
            ),
            // Busy is already covered by the incoming-reply log plus the
            // immediate resend; failed resends stay silent, like before.
            RetryNotice::Busy | RetryNotice::ResendFailed(_) => {}
        },
    )
    .map_err(|err| match err {
        RetryError::TimedOut => AppError::device_timeout(),
        RetryError::Disconnected(reason) => {
            clear_link(state);
            AppError::device_disconnected(reason)
        }
        RetryError::LinkLost => AppError::device_not_connected(),
        RetryError::SendFailed(detail) => AppError::internal(detail),
    })?;
    Ok(result_from_reply(reply))
}

fn clear_link(state: &AppState) {
    if let Ok(mut g) = state.link.lock() {
        *g = LinkState::Disconnected;
    }
}

#[tauri::command]
pub async fn list_usb_ports() -> Result<Vec<String>, AppError> {
    tauri::async_runtime::spawn_blocking(crate::transport::usb::list_ports)
        .await
        .map_err(|e| AppError::internal(format!("list_usb_ports task: {e}")))
}

#[tauri::command]
pub async fn connect_usb(
    port: String,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    let port_for_log = port.clone();
    let link = tauri::async_runtime::spawn_blocking(move || UsbLink::connect(&port))
        .await
        .map_err(|e| AppError::internal(format!("connect task: {e}")))?
        .map_err(|e| AppError::usb_open_failed(e.to_string()))?;
    let shared = state.inner().clone();
    {
        let mut g = shared
            .link
            .lock()
            .map_err(|e| AppError::internal(format!("link mutex poisoned: {e}")))?;
        *g = LinkState::Connected(Box::new(ActiveLink::new(
            LinkKind::Usb,
            Arc::new(link),
        )));
    }
    shared
        .logs
        .info("device", format!("USB connected · {port_for_log}"));
    emit_connection_changed(&shared);
    Ok(())
}

#[tauri::command]
pub async fn discover_ble() -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(BleLink::discover)
        .await
        .map_err(|e| AppError::internal(format!("discover task: {e}")))?
        .map_err(|e| AppError::ble_connect_failed(e.to_string()))
}

#[tauri::command]
pub async fn connect_ble(state: State<'_, SharedState>) -> Result<(), AppError> {
    let link = tauri::async_runtime::spawn_blocking(BleLink::connect)
        .await
        .map_err(|e| AppError::internal(format!("BLE connect task: {e}")))?
        .map_err(|e| AppError::ble_connect_failed(e.to_string()))?;
    let shared = state.inner().clone();
    {
        let mut g = shared
            .link
            .lock()
            .map_err(|e| AppError::internal(format!("link mutex poisoned: {e}")))?;
        *g = LinkState::Connected(Box::new(ActiveLink::new(
            LinkKind::Ble,
            Arc::new(link),
        )));
    }
    shared.logs.info("device", "BLE connected · Inkwash");
    emit_connection_changed(&shared);
    Ok(())
}
#[tauri::command]
pub async fn disconnect_device(state: State<'_, SharedState>) -> Result<(), AppError> {
    let shared = state.inner().clone();
    let old = {
        let mut g = shared
            .link
            .lock()
            .map_err(|e| AppError::internal(format!("link mutex poisoned: {e}")))?;
        std::mem::replace(&mut *g, LinkState::Disconnected)
    };
    // Ask the worker to shut down promptly instead of waiting for it to
    // notice its receivers are gone, and flush anything it already
    // delivered (device log noise, in particular) into the log store
    // before the channel drops.
    if let LinkState::Connected(link) = &old {
        link.transport().disconnect();
        while let Some(event) = link.transport().try_recv() {
            if let Event::Log(line) = event {
                shared.logs.info("device-log", line);
            }
        }
    }
    drop(old);
    shared.logs.info("device", "Device disconnected");
    emit_connection_changed(&shared);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStateInfo {
    pub connected: bool,
    pub kind: String,
    pub port: String,
}

#[tauri::command]
pub fn get_connection_state(state: State<'_, SharedState>) -> Result<ConnectionStateInfo, AppError> {
    let shared = state.inner();
    let g = shared
        .link
        .lock()
        .map_err(|e| AppError::internal(format!("link mutex poisoned: {e}")))?;
    Ok(ConnectionStateInfo {
        connected: g.is_connected(),
        kind: g.kind_label().into(),
        port: g.port_label(),
    })
}

#[tauri::command]
pub async fn get_device_status(
    state: State<'_, SharedState>,
) -> Result<DeviceCommandResult, AppError> {
    let shared = state.inner().clone();
    emit_sync_started(&shared, "status");
    let res = tauri::async_runtime::spawn_blocking(move || {
        send_and_wait(&shared, Command::GetStatus)
    })
    .await
    .map_err(|e| AppError::internal(format!("status task: {e}")))?;
    match &res {
        Ok(_) => emit_sync_finished(&state, "status", true, None),
        Err(e) => emit_sync_finished(&state, "status", false, Some(e.message.clone())),
    }
    res
}

#[tauri::command]
pub async fn set_wifi(
    ssid: String,
    password: String,
    state: State<'_, SharedState>,
) -> Result<DeviceCommandResult, AppError> {
    let ssid = ssid.trim().to_string();
    if ssid.is_empty() {
        return Err(AppError::invalid_input("SSID", "must not be empty"));
    }
    let shared = state.inner().clone();
    shared
        .logs
        .info("device", format!("set_wifi: ssid={ssid} (password redacted)"));
    let cmd = Command::SetWifi { ssid, password };
    tauri::async_runtime::spawn_blocking(move || send_and_wait(&shared, cmd))
        .await
        .map_err(|e| AppError::internal(format!("set_wifi task: {e}")))?
}

#[tauri::command]
pub async fn set_server(
    url: String,
    token: String,
    state: State<'_, SharedState>,
) -> Result<DeviceCommandResult, AppError> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err(AppError::invalid_input("URL", "must not be empty"));
    }
    let shared = state.inner().clone();
    shared.logs.info(
        "device",
        format!(
            "set_server: url={url} (token redacted: {})",
            crate::commands::logs::redact_secret(&token)
        ),
    );
    let cmd = Command::SetServer { url, token };
    tauri::async_runtime::spawn_blocking(move || send_and_wait(&shared, cmd))
        .await
        .map_err(|e| AppError::internal(format!("set_server task: {e}")))?
}

#[tauri::command]
pub async fn set_timezone(
    offset_minutes: i16,
    state: State<'_, SharedState>,
) -> Result<DeviceCommandResult, AppError> {
    if !(-12 * 60..=14 * 60).contains(&offset_minutes) || offset_minutes % 15 != 0 {
        return Err(AppError::invalid_input(
            "offset_minutes",
            "must be a multiple of 15 between -12:00 and +14:00",
        ));
    }
    let shared = state.inner().clone();
    shared.logs.info(
        "device",
        format!("set_timezone: offset_minutes={offset_minutes}"),
    );
    let cmd = Command::SetTimezone { offset_minutes };
    tauri::async_runtime::spawn_blocking(move || send_and_wait(&shared, cmd))
        .await
        .map_err(|e| AppError::internal(format!("set_timezone task: {e}")))?
}

#[tauri::command]
pub async fn sync_now(state: State<'_, SharedState>) -> Result<DeviceCommandResult, AppError> {
    let shared = state.inner().clone();
    emit_sync_started(&shared, "sync");
    let res = tauri::async_runtime::spawn_blocking(move || {
        send_and_wait(&shared, Command::SyncNow)
    })
    .await
    .map_err(|e| AppError::internal(format!("sync task: {e}")))?;
    match &res {
        Ok(_) => emit_sync_finished(&state, "sync", true, None),
        Err(e) => emit_sync_finished(&state, "sync", false, Some(e.message.clone())),
    }
    res
}

#[tauri::command]
pub async fn clear_device_alarms(
    state: State<'_, SharedState>,
) -> Result<DeviceCommandResult, AppError> {
    let shared = state.inner().clone();
    shared.logs.warn("device", "clear_device_alarms");
    tauri::async_runtime::spawn_blocking(move || send_and_wait(&shared, Command::ClearAlarms))
        .await
        .map_err(|e| AppError::internal(format!("clear_alarms task: {e}")))?
}

fn emit_connection_changed(state: &Arc<AppState>) {
    let info = {
        let g = match state.link.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        ConnectionStateInfo {
            connected: g.is_connected(),
            kind: g.kind_label().into(),
            port: g.port_label(),
        }
    };
    let _ = state.ctx.emit("connection-changed", &info);
}

fn emit_sync_started(state: &Arc<AppState>, action: &str) {
    let _ = state
        .ctx
        .emit("sync-started", serde_json::json!({ "action": action }));
}

fn emit_sync_finished(state: &State<'_, SharedState>, action: &str, ok: bool, error: Option<String>) {
    let _ = state.ctx.emit(
        "sync-finished",
        serde_json::json!({ "action": action, "ok": ok, "error": error }),
    );
}

#[allow(dead_code)]
fn _unused_app_handle(_h: &AppHandle) {}

#[cfg(test)]
mod retry_driver_tests {
    use super::*;
    use crate::protocol::Reply;

    /// Scripted link: replays queued events in order, then idles. Idle
    /// ticks really sleep for `tick` so silence-triggered resends run on
    /// realistic timings. Counts every send so tests can assert resend
    /// behaviour.
    struct MockLink {
        /// Idle ticks replayed (instantly) before `events`.
        idles_first: usize,
        events: Vec<Event>,
        sends: usize,
        fail_sends_from: Option<usize>,
    }

    impl MockLink {
        fn new(events: Vec<Event>) -> Self {
            Self {
                idles_first: 0,
                events,
                sends: 0,
                fail_sends_from: None,
            }
        }
    }

    impl RetryLink for MockLink {
        fn send(&mut self, _request_id: &str, _command: &Command) -> Result<(), String> {
            if matches!(self.fail_sends_from, Some(from) if self.sends >= from) {
                return Err("worker gone".into());
            }
            self.sends += 1;
            Ok(())
        }

        fn recv(&mut self, tick: Duration) -> Tick {
            if self.idles_first > 0 {
                self.idles_first -= 1;
                std::thread::sleep(tick);
                return Tick::Idle;
            }
            match self.events.is_empty() {
                true => Tick::Idle,
                false => Tick::Event(self.events.remove(0)),
            }
        }
    }

    #[test]
    fn resends_on_idle_until_reply() {
        // Silence long enough for two retry intervals, then the answer.
        let mut link = MockLink::new(vec![Event::Reply(Some("r1".into()), Reply::Ok)]);
        link.idles_first = 2;
        let reply = drive_request(
            &mut link,
            "r1",
            &Command::GetStatus,
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(15),
            true,
            &mut |_| {},
        )
        .expect("reply expected");
        assert!(matches!(reply, Reply::Ok));
        // Initial send plus at least one silence-triggered resend.
        assert!(link.sends >= 2, "expected resends, got {}", link.sends);
    }

    #[test]
    fn no_idle_resend_when_disabled() {
        // BLE semantics: silence never triggers a resend.
        let mut link = MockLink::new(vec![]);
        let result = drive_request(
            &mut link,
            "r1",
            &Command::GetStatus,
            Instant::now() + Duration::from_millis(60),
            Duration::from_millis(5),
            false,
            &mut |_| {},
        );
        assert!(matches!(result, Err(RetryError::TimedOut)));
        assert_eq!(link.sends, 1);
    }

    #[test]
    fn busy_reply_is_retried_with_same_id() {
        let mut link = MockLink::new(vec![
            Event::Reply(Some("r1".into()), Reply::Busy),
            Event::Reply(Some("r1".into()), Reply::Ok),
        ]);
        let mut busy_seen = false;
        let reply = drive_request(
            &mut link,
            "r1",
            &Command::GetStatus,
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(15),
            false,
            &mut |notice| {
                if matches!(notice, RetryNotice::Busy) {
                    busy_seen = true;
                }
            },
        )
        .expect("final reply expected");
        assert!(matches!(reply, Reply::Ok));
        assert!(busy_seen);
        assert_eq!(link.sends, 2, "initial + one busy-triggered resend");
    }

    #[test]
    fn stale_replies_are_ignored_not_answered() {
        let mut link = MockLink::new(vec![
            Event::Reply(Some("other-request".into()), Reply::Ok),
            Event::Reply(None, Reply::Ok), // no id at all: accepted on trust
        ]);
        let mut stale = Vec::new();
        let reply = drive_request(
            &mut link,
            "r1",
            &Command::GetStatus,
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(15),
            false,
            &mut |notice| {
                if let RetryNotice::StaleReply(id) = notice {
                    stale.push(id.clone());
                }
            },
        )
        .expect("final reply expected");
        assert!(matches!(reply, Reply::Ok));
        assert_eq!(stale, vec!["other-request".to_string()]);
        assert_eq!(link.sends, 1, "stale replies must not trigger a resend");
    }

    #[test]
    fn disconnect_surfaces_as_error() {
        let mut link = MockLink::new(vec![Event::Disconnected("read failed: boom".into())]);
        let result = drive_request(
            &mut link,
            "r1",
            &Command::GetStatus,
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(15),
            false,
            &mut |_| {},
        );
        assert!(matches!(
            result,
            Err(RetryError::Disconnected(reason)) if reason.contains("boom")
        ));
    }

    #[test]
    fn first_send_failure_aborts_immediately() {
        let mut link = MockLink::new(vec![]);
        link.fail_sends_from = Some(0);
        let result = drive_request(
            &mut link,
            "r1",
            &Command::GetStatus,
            Instant::now() + Duration::from_secs(1),
            Duration::from_millis(15),
            true,
            &mut |_| {},
        );
        assert!(matches!(
            result,
            Err(RetryError::SendFailed(reason)) if reason.contains("worker gone")
        ));
    }
}
