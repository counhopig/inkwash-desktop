//! Entry point. CLI dispatch happens first; everything else launches
//! the Tauri window via `desktop::run`.

mod commands;
mod desktop;
mod error;
mod protocol;
mod server;
mod state;
mod transport;

/// `inkwash-desktop --status <serial-port> [timeout-seconds]`: headless USB status check,
/// useful for verifying a connection without going through the GUI (e.g.
/// scripting, or a machine with no display). Everything else launches the
/// normal window.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--ble-scan" {
        match transport::ble::BleLink::discover() {
            Ok(true) => println!("Inkwash BLE advertisement found"),
            Ok(false) => println!("Inkwash BLE advertisement not found"),
            Err(err) => {
                eprintln!("BLE scan failed: {err:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.len() == 2 && args[1] == "--ble-list" {
        match transport::ble::BleLink::scan_report() {
            Ok(report) if report.is_empty() => println!("No BLE advertisements received"),
            Ok(report) => report.iter().for_each(|line| println!("{line}")),
            Err(err) => {
                eprintln!("BLE scan failed: {err:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    if (args.len() == 3 || args.len() == 4) && args[1] == "--status" {
        let timeout = args
            .get(3)
            .and_then(|value| value.parse().ok())
            .unwrap_or(35);
        cli_usb_command(&args[2], timeout, CliAction::Status);
        return;
    }
    if (args.len() == 3 || args.len() == 4) && args[1] == "--sync" {
        let timeout = args
            .get(3)
            .and_then(|value| value.parse().ok())
            .unwrap_or(45);
        cli_usb_command(&args[2], timeout, CliAction::Sync);
        return;
    }
    if (args.len() == 3 || args.len() == 4) && args[1] == "--rtc-sync" {
        let timeout = args
            .get(3)
            .and_then(|value| value.parse().ok())
            .unwrap_or(35);
        cli_usb_command(&args[2], timeout, CliAction::RtcSync);
        return;
    }

    desktop::run();
}

#[derive(Clone, Copy)]
enum CliAction {
    Status,
    Sync,
    RtcSync,
}

impl CliAction {
    fn command(self) -> protocol::Command {
        match self {
            Self::Status => protocol::Command::GetStatus,
            Self::Sync => protocol::Command::SyncNow,
            Self::RtcSync => {
                let epoch_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock is before Unix epoch")
                    .as_secs();
                protocol::Command::SetRtc { epoch_secs }
            }
        }
    }
}

fn cli_usb_command(port: &str, timeout_seconds: u64, action: CliAction) {
    use commands::device::{drive_request, RetryError, RetryLink, RetryNotice, Tick};
    use std::sync::mpsc::RecvTimeoutError;
    use transport::{usb::UsbLink, Transport};

    let link = match UsbLink::connect(port) {
        Ok(link) => link,
        Err(err) => {
            eprintln!("failed to open {port}: {err}");
            std::process::exit(1);
        }
    };

    struct CliUsbLink(UsbLink);

    impl RetryLink for CliUsbLink {
        fn send(&mut self, request_id: &str, command: &protocol::Command) -> Result<(), String> {
            Transport::send(&self.0, request_id, command.clone()).map_err(|e| e.to_string())
        }

        fn recv(&mut self, tick: std::time::Duration) -> Tick {
            match Transport::recv_timeout(&self.0, tick) {
                Ok(event) => Tick::Event(event),
                Err(RecvTimeoutError::Timeout) => Tick::Idle,
                Err(RecvTimeoutError::Disconnected) => Tick::Lost,
            }
        }
    }

    let request_id = protocol::next_request_id();
    // Opening the ESP32-S3 USB Serial/JTAG port may reset the board. Boot
    // can then spend about 25 seconds attempting Wi-Fi before the Home loop
    // starts polling commands, so five seconds produces a misleading
    // timeout - hence the generous default and the shared driver's resends.
    let result = drive_request(
        &mut CliUsbLink(link),
        &request_id,
        &action.command(),
        std::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds),
        std::time::Duration::from_secs(2),
        true,
        &mut |notice| match notice {
            RetryNotice::DeviceLog(line) => println!("(log) {line}"),
            RetryNotice::StaleReply(id) => {
                println!("(ignoring reply for stale request id {id})");
            }
            RetryNotice::Busy => println!("(device busy showing a reminder; retrying)"),
            RetryNotice::ResendFailed(err) => eprintln!("retry send failed: {err}"),
            RetryNotice::IncomingReply(_) => {}
        },
    );

    match result {
        Ok(reply) => println!("{reply:?}"),
        Err(RetryError::TimedOut) => {
            eprintln!("timed out waiting for a reply");
            std::process::exit(1);
        }
        Err(RetryError::Disconnected(reason)) => {
            eprintln!("disconnected: {reason}");
            std::process::exit(1);
        }
        Err(RetryError::LinkLost) => {
            eprintln!("worker thread gone");
            std::process::exit(1);
        }
        Err(RetryError::SendFailed(err)) => {
            eprintln!("send failed: {err}");
            std::process::exit(1);
        }
    }
}
