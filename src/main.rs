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

    desktop::run();
}

#[derive(Clone, Copy)]
enum CliAction {
    Status,
    Sync,
}

impl CliAction {
    fn command(self) -> protocol::Command {
        match self {
            Self::Status => protocol::Command::GetStatus,
            Self::Sync => protocol::Command::SyncNow,
        }
    }
}

fn cli_usb_command(port: &str, timeout_seconds: u64, action: CliAction) {
    use transport::usb::{UsbEvent, UsbLink};

    let link = match UsbLink::connect(port) {
        Ok(link) => link,
        Err(err) => {
            eprintln!("failed to open {port}: {err}");
            std::process::exit(1);
        }
    };
    let request_id = protocol::next_request_id();
    if let Err(err) = link.send(&request_id, action.command()) {
        eprintln!("send failed: {err}");
        std::process::exit(1);
    }

    // Opening the ESP32-S3 USB Serial/JTAG port may reset the board. Boot can
    // then spend about 25 seconds attempting Wi-Fi before the Home loop starts
    // polling commands, so five seconds produces a misleading timeout.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_seconds);
    let mut next_send = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match link
            .event_rx
            .recv_timeout(std::time::Duration::from_millis(200))
        {
            Ok(UsbEvent::Reply(id, reply)) => {
                match protocol::classify_reply(&request_id, id.as_deref(), &reply) {
                    protocol::ReplyDecision::Stale => {
                        println!(
                            "(ignoring reply for stale request id {})",
                            id.as_deref().unwrap_or("?")
                        );
                        continue;
                    }
                    protocol::ReplyDecision::Busy => {
                        println!("(device busy showing a reminder; retrying)");
                        if let Err(err) = link.send(&request_id, action.command()) {
                            eprintln!("retry send failed: {err}");
                        }
                        next_send = std::time::Instant::now() + std::time::Duration::from_secs(2);
                        continue;
                    }
                    protocol::ReplyDecision::Done => {
                        println!("{reply:?}");
                        return;
                    }
                }
            }
            Ok(UsbEvent::Log(line)) => println!("(log) {line}"),
            Ok(UsbEvent::Disconnected(reason)) => {
                eprintln!("disconnected: {reason}");
                std::process::exit(1);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if std::time::Instant::now() >= next_send {
                    if let Err(err) = link.send(&request_id, action.command()) {
                        eprintln!("retry send failed: {err}");
                    }
                    next_send = std::time::Instant::now() + std::time::Duration::from_secs(2);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("worker thread gone");
                std::process::exit(1);
            }
        }
    }
    eprintln!("timed out waiting for a reply");
    std::process::exit(1);
}
