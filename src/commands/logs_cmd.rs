//! Tauri command surface for the log store.
//!
//! All commands take `State<'_, SharedState>` (the `Arc<AppState>`
//! registered with Tauri) and reach into `state.logs` to read or
//! clear the in-memory view.

use tauri::State;

use crate::commands::logs::LogEntry;
use crate::desktop::SharedState;
use crate::error::AppError;

#[tauri::command]
pub fn read_logs(state: State<'_, SharedState>) -> Result<Vec<LogEntry>, AppError> {
    Ok(state.logs.entries())
}

/// Clear the in-memory view. Does NOT delete the file on disk - the
/// file is the permanent record (see migration plan §8.2).
#[tauri::command]
pub fn clear_logs(state: State<'_, SharedState>) -> Result<(), AppError> {
    state.logs.clear_view();
    Ok(())
}

#[tauri::command]
pub fn log_file_path(state: State<'_, SharedState>) -> Result<std::path::PathBuf, AppError> {
    Ok(state.logs.path())
}

#[tauri::command]
pub fn log_dir(state: State<'_, SharedState>) -> Result<std::path::PathBuf, AppError> {
    Ok(state.logs.dir())
}

#[tauri::command]
pub fn open_log_folder(state: State<'_, SharedState>) -> Result<(), AppError> {
    let dir = state.logs.dir();
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&dir).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(&dir).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&dir).spawn()
    };
    result.map_err(|e| AppError::internal(format!("open log folder: {e}")))?;
    Ok(())
}

#[tauri::command]
pub fn export_log(state: State<'_, SharedState>) -> Result<String, AppError> {
    let src = state.logs.path();
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let downloads = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Downloads"))
        .or_else(|_| std::env::current_dir())
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    let dest = downloads.join(format!("inkwash-desktop-export-{epoch}.log"));
    std::fs::copy(&src, &dest).map_err(|e| AppError::internal(format!("export log: {e}")))?;
    Ok(dest.to_string_lossy().to_string())
}
