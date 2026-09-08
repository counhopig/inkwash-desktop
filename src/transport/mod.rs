//! Transport abstraction shared by the USB serial and BLE links
//! (`transport::usb` / `transport::ble`).
//!
//! Both links expose the same session shape - push a command with a
//! correlation id, wait for [`Event`]s on the same link afterwards - so
//! callers (`state::LinkState`, `commands::device`, the headless CLI in
//! `main.rs`) program against the [`Transport`] trait object and never
//! care which radio is underneath. The two workers themselves also share
//! one polling-loop skeleton ([`run_worker_loop`]); only the
//! hardware-specific read/write halves live in their own modules.

pub mod ble;
pub mod usb;

use std::sync::mpsc;
use std::time::Duration;

use crate::protocol::{Command, Reply};

/// How often a worker's polling loop wakes up to re-check the outgoing
/// command queue when no inbound traffic arrives. USB uses it as its
/// blocking read timeout; BLE as the notification-await timeout.
pub const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// One occurrence on a transport, delivered from the worker thread to
/// whoever holds the link. Previously each transport had its own
/// structurally identical event enum (`UsbEvent`/`BleEvent`); they are
/// unified here so trait-object callers can match a single type.
#[derive(Debug, Clone)]
pub enum Event {
    /// A protocol reply arrived. `id` is the reply's correlation id (see
    /// `protocol::decode_reply`), `None` if the device didn't echo one
    /// back.
    Reply(Option<String>, Reply),
    /// Device log noise - any inbound traffic that isn't a reply - kept
    /// so the UI can show it for debugging.
    Log(String),
    /// The port/stream closed or errored; the worker thread has exited.
    Disconnected(String),
}

/// A connected device link: one worker thread plus the channels to talk
/// to it. All methods take `&self`; the receiver side is guarded by an
/// internal mutex so a link can be polled without holding whatever outer
/// lock stores it (see `state.rs` invariant 1).
pub trait Transport: Send + Sync {
    /// Queue `cmd` for the worker, tagged with the request correlation
    /// `id`. Generate ids with `protocol::next_request_id()` and reuse
    /// one across resends of the same logical request.
    fn send(&self, id: &str, cmd: Command) -> anyhow::Result<()>;

    /// Waits up to `timeout` for the next [`Event`] from the worker.
    fn recv_timeout(&self, timeout: Duration) -> Result<Event, mpsc::RecvTimeoutError>;

    /// Non-blocking peek at the next [`Event`], if one is queued.
    fn try_recv(&self) -> Option<Event> {
        self.recv_timeout(Duration::ZERO).ok()
    }

    /// Asks the worker thread to shut down. The worker notices within
    /// one [`POLL_INTERVAL`] tick and reports [`Event::Disconnected`];
    /// dropping the link has always torn it down eventually too (the
    /// event channel disconnects), this just makes it prompt and
    /// explicit.
    fn disconnect(&self);
}

/// One chunk of inbound traffic, already classified by the transport's
/// framing rules: USB strips the `<<IW ` reply prefix line-by-line, BLE
/// treats each notification as a reply candidate.
pub(crate) enum Inbound {
    /// Text that should be parsed as a protocol reply payload.
    Reply(String),
    Log(String),
}

/// Result of trying to push one encoded command onto the device.
pub(crate) enum WriteOutcome {
    Sent,
    /// Unrecoverable write error; the worker must shut down.
    Fatal(String),
}

/// The hardware-specific half of a transport worker: framed writes plus
/// one polled inbound read per tick. Implemented by small adapters in
/// `usb.rs` (blocking `serialport` I/O) and `ble.rs` (async GATT I/O
/// driven through the link's dedicated Tokio runtime) so that
/// [`run_worker_loop`] below is the single copy of the polling skeleton
/// both workers used to carry separately.
pub(crate) trait PollSource {
    /// Waits up to roughly [`POLL_INTERVAL`] for the next inbound chunk.
    /// `Ok(None)` means "nothing this tick" (a timeout is normal, not an
    /// error); `Err` is fatal and ends the worker.
    fn poll(&mut self) -> Result<Option<Inbound>, String>;

    /// Writes one encoded command payload (framing is the adapter's
    /// job: USB adds `>>IW …\n`, BLE writes bare JSON via a GATT write).
    fn write_command(&mut self, payload: &str) -> WriteOutcome;
}

/// The shared ~200ms polling worker loop used by every transport.
///
/// Each tick, in order:
///
/// 1. honour a pending [`Transport::disconnect`] request,
/// 2. drain every queued outgoing command into the device,
/// 3. poll for one inbound chunk and dispatch it as an [`Event`].
///
/// Any fatal write/read outcome sends [`Event::Disconnected`] and ends
/// the thread; a failed `Reply` delivery (receiver gone) just exits
/// quietly like the per-transport loops did before unification.
pub(crate) fn run_worker_loop(
    io: &mut dyn PollSource,
    cmd_rx: mpsc::Receiver<(String, Command)>,
    event_tx: &mpsc::Sender<Event>,
    stop: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;

    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = event_tx.send(Event::Disconnected("link closed".into()));
            return;
        }

        // Drain any queued outgoing commands first - cheap, and keeps
        // command latency low relative to POLL_INTERVAL.
        while let Ok((id, cmd)) = cmd_rx.try_recv() {
            let payload = crate::protocol::encode_command(&cmd, &id);
            match io.write_command(&payload) {
                WriteOutcome::Sent => {}
                WriteOutcome::Fatal(reason) => {
                    let _ = event_tx.send(Event::Disconnected(reason));
                    return;
                }
            }
        }

        match io.poll() {
            Ok(Some(Inbound::Log(line))) => {
                let _ = event_tx.send(Event::Log(line));
            }
            Ok(Some(Inbound::Reply(text))) => match crate::protocol::decode_reply(&text) {
                Ok((id, reply)) => {
                    if event_tx.send(Event::Reply(id, reply)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = event_tx.send(Event::Log(format!("(unparseable reply: {err})")));
                }
            },
            Ok(None) => {} // nothing this tick; loop back to the command queue
            Err(reason) => {
                let _ = event_tx.send(Event::Disconnected(reason));
                return;
            }
        }
    }
}
