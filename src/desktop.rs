//! Tauri builder + setup. The actual command implementations live in
//! `crate::commands` and are re-exported here for the `invoke_handler`
//! macro.

use std::sync::Arc;

use tauri::Manager;

use crate::commands;
use crate::state::AppState;

pub type SharedState = Arc<AppState>;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(SharedState::new(AppState::new(handle)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::device::list_usb_ports,
            commands::device::connect_usb,
            commands::device::discover_ble,
            commands::device::connect_ble,
            commands::device::disconnect_device,
            commands::device::get_connection_state,
            commands::device::get_device_status,
            commands::device::set_wifi,
            commands::device::set_server,
            commands::device::set_timezone,
            commands::device::sync_now,
            commands::device::clear_device_alarms,
            commands::server::list_devices,
            commands::server::register_device,
            commands::server::delete_device,
            commands::server::create_alarm,
            commands::server::update_alarm,
            commands::server::delete_alarm,
            commands::server::clear_alarms,
            commands::server::create_todo,
            commands::server::update_todo,
            commands::server::delete_todo,
            commands::server::clear_todos,
            commands::server::list_content,
            commands::server::create_webhook_channel,
            commands::server::delete_channel,
            commands::server::rotate_channel_token,
            commands::server::delete_inbox_item,
            commands::server::clear_inbox,
            commands::storage::load_admin_token,
            commands::storage::save_admin_token,
            commands::logs_cmd::read_logs,
            commands::logs_cmd::clear_logs,
            commands::logs_cmd::log_file_path,
            commands::logs_cmd::log_dir,
            commands::logs_cmd::open_log_folder,
            commands::logs_cmd::export_log,
            commands::scan::scan_wifi_networks,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Inkwash Desktop");
}
