//! In-memory ring buffer of log entries plus a mirror to disk.
//!
//! Every entry is appended in three places atomically-from-the-user's-
//! point-of-view:
//!
//! * stderr (so `cargo run -- --ble-scan` and the GUI both see it)
//! * a per-launch `<log_dir>/inkwash-desktop-<epoch>.log` file
//! * `device-log` Tauri event (so the Logs page can render in real time
//!   without polling every 100ms - see migration plan §8.3)
//!
//! `log_dir` lives in a platform data directory *outside* the project
//! tree: `tauri dev` watches the project root and would otherwise
//! restart in an infinite loop every time a log line is flushed.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use ts_rs::TS;

const LOG_PREFIX: &str = "inkwash-desktop-";
const LOG_SUFFIX: &str = ".log";
const MAX_INMEM_ENTRIES: usize = 4_000;

fn platform_log_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Logs/inkwash-desktop");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(base).join("inkwash-desktop").join("logs");
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local/share/inkwash-desktop/logs");
        }
    }
    std::env::temp_dir().join("inkwash-desktop")
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export, export_to = "../src-ui/lib/generated/")]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    #[allow(dead_code)]
    Error,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../src-ui/lib/generated/")]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    #[ts(type = "number")]
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

pub struct LogStore {
    inner: Mutex<LogInner>,
    ctx: AppHandle,
}

struct LogInner {
    entries: Vec<LogEntry>,
    file: Option<File>,
    path: PathBuf,
    bytes_written: u64,
}

impl LogStore {
    pub fn new(ctx: AppHandle) -> Self {
        let dir = platform_log_dir();
        let _ = fs::create_dir_all(&dir);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("{LOG_PREFIX}{epoch}{LOG_SUFFIX}"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        Self {
            inner: Mutex::new(LogInner {
                entries: Vec::new(),
                file,
                path,
                bytes_written: 0,
            }),
            ctx,
        }
    }

    pub fn info(&self, source: &str, message: impl AsRef<str>) {
        self.append(LogLevel::Info, source, message.as_ref());
    }
    pub fn warn(&self, source: &str, message: impl AsRef<str>) {
        self.append(LogLevel::Warn, source, message.as_ref());
    }

    pub fn append(&self, level: LogLevel, source: &str, message: &str) {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let entry = LogEntry {
            timestamp_ms,
            level,
            source: source.to_string(),
            message: message.to_string(),
        };

        // Always mirror to stderr (CLI mode picks this up).
        eprintln!(
            "[{}] {} {}: {}",
            entry.timestamp_ms,
            level_tag(level),
            entry.source,
            entry.message
        );

        let (snapshot, path) = {
            let mut g = self.inner.lock().expect("log mutex poisoned");
            g.entries.push(entry.clone());
            if g.entries.len() > MAX_INMEM_ENTRIES {
                let drop = g.entries.len() - MAX_INMEM_ENTRIES;
                g.entries.drain(..drop);
            }
            if let Some(f) = g.file.as_mut() {
                let line = format!(
                    "[{}] {} {}: {}\n",
                    entry.timestamp_ms,
                    level_tag(level),
                    entry.source,
                    entry.message
                );
                let bytes = line.len();
                if f.write_all(line.as_bytes()).is_ok() {
                    g.bytes_written += bytes as u64;
                }
            }
            (entry.clone(), g.path.clone())
        };

        // Emit to GUI subscribers. A failure here just means no one is
        // listening yet (startup race) and is not worth escalating.
        let _ = self.ctx.emit("device-log", &snapshot);
        // Also keep the path available so the Logs page can offer
        // "Open log folder" without re-discovering it.
        let _ = path;
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.inner
            .lock()
            .expect("log mutex poisoned")
            .entries
            .clone()
    }

    /// Clear the in-memory view. Does NOT delete or truncate the file
    /// on disk - that's the whole point of the "Clear view" button in
    /// the Logs page toolbar (see migration plan §8.2).
    pub fn clear_view(&self) {
        self.inner
            .lock()
            .expect("log mutex poisoned")
            .entries
            .clear();
    }

    pub fn path(&self) -> PathBuf {
        self.inner.lock().expect("log mutex poisoned").path.clone()
    }

    /// Directory containing the current log file (used for "Open log
    /// folder" on the Logs page toolbar).
    pub fn dir(&self) -> PathBuf {
        self.inner
            .lock()
            .expect("log mutex poisoned")
            .path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(platform_log_dir)
    }
}

fn level_tag(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

/// Truncate a sensitive value for logging. We never want a Wi-Fi
/// password, admin token, or device token sitting in the on-disk log
/// even in a debug build. The leading + trailing characters give a
/// human enough to identify which token triggered a failure.
pub fn redact_secret(value: &str) -> String {
    let len = value.chars().count();
    if len <= 8 {
        return "****".into();
    }
    let head: String = value.chars().take(4).collect();
    let tail: String = value.chars().skip(len - 4).collect();
    format!("{head}\u{2026}{tail}")
}

/// Normalise a server URL the same way the CLI and ServerClient do -
/// strip a trailing slash, prepend `http://` if no scheme was given.
pub fn normalise_server_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    };
    let no_trailing = with_scheme.trim_end_matches('/').to_string();
    if no_trailing.is_empty() {
        None
    } else {
        Some(no_trailing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_short_secrets_completely() {
        assert_eq!(redact_secret("ab"), "****");
        assert_eq!(redact_secret("12345678"), "****");
    }

    #[test]
    fn redacts_long_secrets_to_head_and_tail() {
        assert_eq!(redact_secret("abcdefghijklmnop"), "abcd\u{2026}mnop");
    }

    #[test]
    fn normalises_url_with_trailing_slash() {
        assert_eq!(
            normalise_server_url("http://192.168.1.10:8080/").unwrap(),
            "http://192.168.1.10:8080"
        );
    }

    #[test]
    fn normalises_url_without_scheme() {
        assert_eq!(
            normalise_server_url("192.168.1.10:8080").unwrap(),
            "http://192.168.1.10:8080"
        );
    }

    #[test]
    fn rejects_empty_url() {
        assert!(normalise_server_url("").is_none());
        assert!(normalise_server_url("   ").is_none());
    }
}
