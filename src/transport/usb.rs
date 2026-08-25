//! USB serial transport, matching `inkwash/docs/control-protocol.md`'s
//! framing: commands go out as `>>IW {json}\n`, replies come back as
//! `<<IW {json}\n` on the same line-oriented stream that also carries the
//! device's ordinary `log::info!` output - any line without the `<<IW `
//! prefix is just log noise from this reader's point of view and is
//! surfaced as a [`Event::Log`] instead of discarded, so the UI can show
//! it for debugging.
//!
//! The worker thread runs the shared polling skeleton from
//! `transport::run_worker_loop`; only the blocking serial I/O below is
//! USB-specific.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::protocol::Command;
use crate::transport::{
    run_worker_loop, Event, Inbound, PollSource, Transport, WriteOutcome, POLL_INTERVAL,
};

const COMMAND_PREFIX: &str = ">>IW ";
const REPLY_PREFIX: &str = "<<IW ";
const BAUD_RATE: u32 = 115_200;
/// `serialport` shares one timeout value for both directions, so a write
/// borrows this longer one for its duration and restores
/// [`POLL_INTERVAL`] right after. 200ms was too tight: a resend landing
/// during the device's brief but interrupt-heavy Wi-Fi association window
/// could trip it and tear down the whole connection over a transient
/// stall, not an actual disconnect.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub struct UsbLink {
    cmd_tx: mpsc::Sender<(String, Command)>,
    event_rx: Mutex<mpsc::Receiver<Event>>,
    stop: Arc<AtomicBool>,
}

impl UsbLink {
    /// Opens `port_name` and spawns the reader/writer worker thread.
    pub fn connect(port_name: &str) -> anyhow::Result<Self> {
        let mut port = serialport::new(port_name, BAUD_RATE)
            .timeout(POLL_INTERVAL)
            .open()
            .map_err(|e| anyhow::anyhow!("failed to open {port_name}: {e}"))?;

        // ESP32-S3 USB Serial/JTAG uses modem-control lines for reset and
        // download-mode entry. Explicitly release both after opening; host
        // defaults vary and can otherwise leave the device silent while the
        // port itself still appears to have opened successfully.
        port.write_data_terminal_ready(false)
            .map_err(|e| anyhow::anyhow!("failed to release DTR on {port_name}: {e}"))?;
        port.write_request_to_send(false)
            .map_err(|e| anyhow::anyhow!("failed to release RTS on {port_name}: {e}"))?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<(String, Command)>();
        let (event_tx, event_rx) = mpsc::channel::<Event>();
        let stop = Arc::new(AtomicBool::new(false));

        let worker_stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut adapter = UsbAdapter {
                port,
                line_buf: Vec::new(),
                read_buf: [0u8; 256],
            };
            run_worker_loop(&mut adapter, cmd_rx, &event_tx, &worker_stop);
        });

        Ok(Self {
            cmd_tx,
            event_rx: Mutex::new(event_rx),
            stop,
        })
    }
}

impl Transport for UsbLink {
    fn send(&self, id: &str, cmd: Command) -> anyhow::Result<()> {
        self.cmd_tx
            .send((id.to_string(), cmd))
            .map_err(|_| anyhow::anyhow!("USB worker thread is gone"))
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<Event, mpsc::RecvTimeoutError> {
        self.event_rx
            .lock()
            .expect("usb event_rx mutex poisoned")
            .recv_timeout(timeout)
    }

    fn disconnect(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// USB half of the worker: framed writes with the longer write timeout,
/// plus byte-to-line reassembly for inbound traffic.
struct UsbAdapter {
    port: Box<dyn serialport::SerialPort>,
    line_buf: Vec<u8>,
    read_buf: [u8; 256],
}

impl PollSource for UsbAdapter {
    fn poll(&mut self) -> Result<Option<Inbound>, String> {
        loop {
            // Deliver any complete lines already reassembled before
            // touching the port again, so a burst of several lines in one
            // read chunk is still processed in order across ticks.
            if let Some(pos) = self.line_buf.iter().position(|&b| b == b'\n') {
                let mut line_bytes = self.line_buf.split_off(pos + 1);
                std::mem::swap(&mut line_bytes, &mut self.line_buf);
                let line = String::from_utf8_lossy(&line_bytes)
                    .trim_end_matches('\r')
                    .to_string();
                if let Some(json) = line.strip_prefix(REPLY_PREFIX) {
                    return Ok(Some(Inbound::Reply(json.to_string())));
                } else if !line.is_empty() {
                    return Ok(Some(Inbound::Log(line)));
                }
                continue; // blank line, keep scanning
            }

            match self.port.read(&mut self.read_buf) {
                Ok(0) => return Ok(None),
                Ok(n) => self.line_buf.extend_from_slice(&self.read_buf[..n]),
                // A read timeout with nothing available is the normal "no
                // data yet" case for a port opened with a fixed timeout,
                // not an error - only genuine I/O errors end the worker.
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => return Ok(None),
                Err(e) => return Err(format!("read failed: {e}")),
            }
        }
    }

    fn write_command(&mut self, payload: &str) -> WriteOutcome {
        let line = format!("{COMMAND_PREFIX}{payload}\n");
        if let Err(e) = self.port.set_timeout(WRITE_TIMEOUT) {
            return WriteOutcome::Fatal(format!("failed to set write timeout: {e}"));
        }
        let write_result = self.port.write_all(line.as_bytes());
        // Restore the short poll timeout regardless of how the write went,
        // so a failed write doesn't also wedge the read side at the long
        // timeout for however long this thread lives.
        if let Err(e) = self.port.set_timeout(POLL_INTERVAL) {
            return WriteOutcome::Fatal(format!("failed to restore read timeout: {e}"));
        }
        match write_result {
            Ok(()) => WriteOutcome::Sent,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                // The device didn't drain its input in time. Most likely
                // it's still busy executing an earlier command -
                // `control::dispatch` is synchronous and the firmware stops
                // polling USB for the whole duration of a slow one like
                // sync_now, which can run well past this write's timeout.
                // That's not a disconnect: drop this one write attempt
                // (nothing was sent - `write()` waits for POLLOUT before
                // attempting any bytes, so this can't have left a partial,
                // framing-corrupting line on the wire) and let the caller's
                // own resend timer try again. A real disconnect still
                // surfaces via the read side.
                WriteOutcome::Busy(format!(
                    "(write timed out, device likely busy; will retry: {e})"
                ))
            }
            Err(e) => WriteOutcome::Fatal(format!("write failed: {e}")),
        }
    }
}

/// Lists available serial port names for the connection picker.
pub fn list_ports() -> Vec<String> {
    let ports = serialport::available_ports().unwrap_or_default();
    let mut espressif: Vec<String> = ports
        .iter()
        .filter(|port| {
            matches!(
                &port.port_type,
                serialport::SerialPortType::UsbPort(info) if info.vid == 0x303a
            )
        })
        .map(|port| port.port_name.clone())
        .collect();
    if espressif.is_empty() {
        espressif = ports
            .into_iter()
            .filter(|port| matches!(port.port_type, serialport::SerialPortType::UsbPort(_)))
            .map(|port| port.port_name)
            .collect();
    }
    espressif.sort();
    espressif
}
