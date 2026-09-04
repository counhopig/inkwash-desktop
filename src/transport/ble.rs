//! BLE transport, matching `inkwash/docs/control-protocol.md`'s BLE
//! framing section: commands are written to the write characteristic as
//! plain JSON (no line framing needed - GATT writes are already
//! message-delimited), replies arrive as notifications on the separate
//! notify characteristic. Runs its own Tokio runtime on a dedicated
//! thread, since `btleplug` is async-only and the Tauri runtime's
//! tokio-blocking boundary cannot host it directly - same "worker
//! thread + `std::sync::mpsc` channel" shape as [`super::usb`], just
//! with an async worker body instead of a blocking one.
//!
//! The worker thread runs the shared polling skeleton from
//! `transport::run_worker_loop`; each tick's GATT read/write is driven
//! through the dedicated runtime with `block_on` from the owning thread
//! (legal there, since the loop itself is not inside an async context).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Manager, Peripheral};
use futures::StreamExt;
use uuid::Uuid;

use crate::protocol::Command;
use crate::transport::{
    run_worker_loop, Event, Inbound, PollSource, Transport, WriteOutcome, POLL_INTERVAL,
};

const SERVICE_UUID: &str = "d2c25e50-5e22-48d8-a8b3-34f2f8e2c7d4";
const WRITE_CHAR_UUID: &str = "d2c25e51-5e22-48d8-a8b3-34f2f8e2c7d4";
const NOTIFY_CHAR_UUID: &str = "d2c25e52-5e22-48d8-a8b3-34f2f8e2c7d4";
/// Advertised device name set in `ble_control.rs::BleControl::start`.
const DEVICE_NAME: &str = "Inkwash";

pub struct BleLink {
    cmd_tx: mpsc::Sender<(String, Command)>,
    event_rx: Mutex<mpsc::Receiver<Event>>,
    stop: Arc<AtomicBool>,
}

impl BleLink {
    /// Diagnostic snapshot of every peripheral CoreBluetooth exposed during
    /// a short scan. Used by the CLI to distinguish filtering bugs from a
    /// scan that receives no advertisements at all.
    pub fn scan_report() -> anyhow::Result<Vec<String>> {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let manager = Manager::new().await?;
            let adapter = manager
                .adapters()
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("no BLE adapter available"))?;
            adapter.start_scan(ScanFilter::default()).await?;
            tokio::time::sleep(Duration::from_secs(5)).await;
            let mut report = Vec::new();
            for peripheral in adapter.peripherals().await? {
                match peripheral.properties().await {
                    Ok(Some(props)) => report.push(format!("{props:?}")),
                    Ok(None) => report.push(format!("{:?}: no properties", peripheral.id())),
                    Err(err) => report.push(format!("{:?}: {err}", peripheral.id())),
                }
            }
            adapter.stop_scan().await.ok();
            Ok(report)
        })
    }

    /// Background-friendly discovery probe. The firmware advertises only
    /// while its BLE Pairing page is open.
    pub fn discover() -> anyhow::Result<bool> {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let manager = Manager::new().await?;
            let adapters = manager.adapters().await?;
            let Some(adapter) = adapters.into_iter().next() else {
                return Err(anyhow::anyhow!(
                    "no BLE adapter is available to this app; check that Bluetooth is on and allow Bluetooth access in System Settings > Privacy & Security > Bluetooth"
                ));
            };
            adapter.start_scan(ScanFilter::default()).await?;
            let found = find_device_with_retries(&adapter, DEVICE_NAME, 8)
                .await
                .is_ok();
            adapter.stop_scan().await.ok();
            Ok(found)
        })
    }

    /// Scans for a device named `DEVICE_NAME`, connects, and subscribes to
    /// the reply characteristic. Blocks the calling thread until connected
    /// or the scan/connect attempt fails - call this from a worker thread
    /// or `tokio::task::spawn_blocking`, not directly from the UI's
    /// `update()`.
    pub fn connect() -> anyhow::Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<(String, Command)>();
        let (event_tx, event_rx) = mpsc::channel::<Event>();
        let (ready_tx, ready_rx) = mpsc::channel::<anyhow::Result<()>>();
        let stop = Arc::new(AtomicBool::new(false));

        // Dedicated thread + dedicated runtime, exactly as before: setup
        // runs inside one `block_on`, then the shared (synchronous)
        // polling loop takes over and drives each GATT operation with its
        // own short `block_on` call.
        let worker_stop = Arc::clone(&stop);
        thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!("tokio runtime init failed: {e}")));
                    return;
                }
            };
            match rt.block_on(connect_and_setup(&event_tx, &ready_tx)) {
                Ok(session) => {
                    let mut adapter = BleAdapter {
                        rt,
                        peripheral: session.peripheral,
                        write_char: session.write_char,
                        notifications: Box::pin(session.notifications),
                    };
                    run_worker_loop(&mut adapter, cmd_rx, &event_tx, &worker_stop);
                }
                Err(e) => {
                    // If setup failed this wakes connect(); if setup had
                    // already succeeded, the receiver is gone and the
                    // disconnect event below is what the UI observes.
                    let _ = ready_tx.send(Err(anyhow::anyhow!("{e}")));
                    let _ = event_tx.send(Event::Disconnected(e.to_string()));
                }
            }
        });

        // The worker signals readiness only after it has actually
        // connected and subscribed (see `connect_and_setup`), so a caller
        // blocking on this knows the link is live before it returns, not
        // just that the thread started.
        ready_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("BLE worker thread exited before connecting"))??;

        Ok(Self {
            cmd_tx,
            event_rx: Mutex::new(event_rx),
            stop,
        })
    }
}

impl Transport for BleLink {
    fn send(&self, id: &str, cmd: Command) -> anyhow::Result<()> {
        self.cmd_tx
            .send((id.to_string(), cmd))
            .map_err(|_| anyhow::anyhow!("BLE worker thread is gone"))
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<Event, mpsc::RecvTimeoutError> {
        self.event_rx
            .lock()
            .expect("ble event_rx mutex poisoned")
            .recv_timeout(timeout)
    }

    fn disconnect(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for BleLink {
    fn drop(&mut self) {
        // Replacing a connected LinkState (for example when switching from
        // USB to BLE) must stop the worker even when the explicit disconnect
        // command was not called first.
        self.disconnect();
    }
}

/// Everything the polling loop needs once GATT setup has completed.
struct BleSession {
    peripheral: Peripheral,
    write_char: btleplug::api::Characteristic,
    notifications:
        std::pin::Pin<Box<dyn futures::Stream<Item = btleplug::api::ValueNotification> + Send>>,
}

/// Scan, connect, subscribe, signal readiness. The pre-loop half of the
/// old monolithic `connect_and_run`; from here the shared
/// `run_worker_loop` owns both directions.
async fn connect_and_setup(
    event_tx: &mpsc::Sender<Event>,
    ready_tx: &mpsc::Sender<anyhow::Result<()>>,
) -> anyhow::Result<BleSession> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let adapter = adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no BLE adapter found"))?;

    adapter.start_scan(ScanFilter::default()).await?;
    let peripheral = find_device(&adapter, DEVICE_NAME).await?;
    adapter.stop_scan().await.ok();

    peripheral.connect().await?;
    peripheral.discover_services().await?;

    let write_uuid = Uuid::parse_str(WRITE_CHAR_UUID)?;
    let notify_uuid = Uuid::parse_str(NOTIFY_CHAR_UUID)?;
    let chars = peripheral.characteristics();
    let write_char = chars
        .iter()
        .find(|c| c.uuid == write_uuid)
        .ok_or_else(|| anyhow::anyhow!("write characteristic not found"))?
        .clone();
    let notify_char = chars
        .iter()
        .find(|c| c.uuid == notify_uuid)
        .ok_or_else(|| anyhow::anyhow!("notify characteristic not found"))?
        .clone();

    peripheral.subscribe(&notify_char).await?;
    let notifications = peripheral.notifications().await?;

    // Report readiness as soon as GATT setup is complete. Previously this
    // was sent only after this long-running loop returned, so the Desktop
    // UI could never receive a live BleLink.
    ready_tx
        .send(Ok(()))
        .map_err(|_| anyhow::anyhow!("BLE connection requester went away"))?;

    let _ = event_tx.send(Event::Log(format!(
        "connected to {DEVICE_NAME} (service {SERVICE_UUID})"
    )));

    Ok(BleSession {
        peripheral,
        write_char,
        notifications,
    })
}

/// BLE half of the worker: bare-JSON GATT writes and notification reads,
/// each driven through the link's dedicated Tokio runtime from the
/// synchronous polling loop.
struct BleAdapter {
    rt: tokio::runtime::Runtime,
    peripheral: Peripheral,
    write_char: btleplug::api::Characteristic,
    notifications:
        std::pin::Pin<Box<dyn futures::Stream<Item = btleplug::api::ValueNotification> + Send>>,
}

impl PollSource for BleAdapter {
    fn poll(&mut self) -> Result<Option<Inbound>, String> {
        let Self {
            rt, notifications, ..
        } = self;
        match poll_notification(rt, notifications) {
            // A timeout with no notification is the normal idle tick.
            Err(_elapsed) => Ok(None),
            Ok(None) => Err("notification stream ended".to_string()),
            Ok(Some(data)) => Ok(Some(Inbound::Reply(
                String::from_utf8_lossy(&data.value).to_string(),
            ))),
        }
    }

    fn write_command(&mut self, payload: &str) -> WriteOutcome {
        match self.rt.block_on(self.peripheral.write(
            &self.write_char,
            payload.as_bytes(),
            WriteType::WithResponse,
        )) {
            Ok(()) => WriteOutcome::Sent,
            Err(e) => WriteOutcome::Fatal(format!("write failed: {e}")),
        }
    }
}

/// Construct the timeout future inside the worker runtime. Tokio's timer
/// constructor looks up the current reactor immediately; constructing it as
/// an argument to `Runtime::block_on` evaluates it on the caller thread,
/// outside that runtime, and panics with "there is no reactor running".
fn poll_notification(
    rt: &tokio::runtime::Runtime,
    notifications: &mut std::pin::Pin<
        Box<dyn futures::Stream<Item = btleplug::api::ValueNotification> + Send>,
    >,
) -> Result<Option<btleplug::api::ValueNotification>, tokio::time::error::Elapsed> {
    rt.block_on(async { tokio::time::timeout(POLL_INTERVAL, notifications.next()).await })
}

async fn find_device(
    adapter: &btleplug::platform::Adapter,
    name: &str,
) -> anyhow::Result<Peripheral> {
    find_device_with_retries(adapter, name, 20).await
}

async fn find_device_with_retries(
    adapter: &btleplug::platform::Adapter,
    name: &str,
    retries: usize,
) -> anyhow::Result<Peripheral> {
    let service_uuid = Uuid::parse_str(SERVICE_UUID)?;
    // A handful of short retries rather than one long wait: advertising
    // packets aren't guaranteed to be seen on the very first scan pass,
    // but this device only advertises while its BLE Pairing screen is
    // open (see `docs/control-protocol.md`'s Lifecycle notes), so we
    // don't want to hang indefinitely if the user hasn't opened it yet.
    for _ in 0..retries {
        for p in adapter.peripherals().await? {
            if let Ok(Some(props)) = p.properties().await {
                // CoreBluetooth does not consistently expose local_name for
                // unpaired peripherals. The advertised service UUID is the
                // stable identity and is also more specific than the name.
                if props.local_name.as_deref() == Some(name)
                    || props.services.contains(&service_uuid)
                {
                    return Ok(p);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(anyhow::anyhow!(
        "no BLE device named '{name}' found after scanning - make sure the device's BLE Pairing screen is open"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_timeout_is_created_inside_worker_runtime() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let mut notifications: std::pin::Pin<
            Box<dyn futures::Stream<Item = btleplug::api::ValueNotification> + Send>,
        > = Box::pin(futures::stream::pending());

        // This must return a normal timeout from a plain OS thread. If the
        // timeout future is constructed before block_on, Tokio panics here
        // because no reactor is current on this thread.
        assert!(poll_notification(&rt, &mut notifications).is_err());
    }
}
