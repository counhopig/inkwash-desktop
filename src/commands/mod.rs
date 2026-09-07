//! Tauri command surface, grouped by responsibility:
//!
//! * `device` - USB/BLE connection lifecycle, sending commands to the
//!   physical Inkwash, polling its status.
//! * `server` - admin HTTP API for `inkwash-server` (devices, alarms,
//!   todos). All of these run on `spawn_blocking` since the underlying
//!   `reqwest` client is blocking.
//! * `scan` - PC-side 2.4 GHz Wi-Fi network scan (CoreWLAN / netsh /
//!   nmcli), independent of the device connection.
//! * `logs_cmd` - log entry read/clear and log-file path discovery.

pub mod device;
pub mod logs;
pub mod logs_cmd;
pub mod scan;
pub mod server;
pub mod storage;
