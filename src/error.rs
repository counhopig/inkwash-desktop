//! Unified error type returned by every Tauri command. The React side
//! must never see a raw `anyhow::Error` chain - it gets a short, human
//! message via `message` and a stable error code via `code` for
//! programmatic handling. The full detail is still preserved on disk
//! via the log store (see `commands::logs`).
//!
//! Error code conventions are documented at the call sites below.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../src-ui/lib/generated/")]
pub struct AppError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn device_not_connected() -> Self {
        Self::new(
            "DEVICE_NOT_CONNECTED",
            "Connect a device first (USB or BLE)",
        )
    }

    pub fn usb_open_failed(detail: impl std::fmt::Display) -> Self {
        Self::new("USB_OPEN_FAILED", "Failed to open USB serial port")
            .with_detail(detail.to_string())
    }

    #[allow(dead_code)]
    pub fn ble_not_found() -> Self {
        Self::new(
            "BLE_NOT_FOUND",
            "No Inkwash BLE device found - make sure its BLE Pairing screen is open",
        )
    }

    pub fn ble_connect_failed(detail: impl std::fmt::Display) -> Self {
        Self::new("BLE_CONNECT_FAILED", "Failed to connect to BLE device")
            .with_detail(detail.to_string())
    }

    pub fn wifi_scan_failed(detail: impl std::fmt::Display) -> Self {
        Self::new("WIFI_SCAN_FAILED", "Wi-Fi scan unavailable").with_detail(detail.to_string())
    }

    pub fn device_timeout() -> Self {
        Self::new("DEVICE_TIMEOUT", "Timed out waiting for the device")
    }

    pub fn device_disconnected(reason: impl Into<String>) -> Self {
        Self::new("DEVICE_DISCONNECTED", "Device disconnected").with_detail(reason.into())
    }

    pub fn server_unreachable(detail: impl std::fmt::Display) -> Self {
        Self::new("SERVER_UNREACHABLE", "Cannot reach the server").with_detail(detail.to_string())
    }

    pub fn server_unauthorized() -> Self {
        Self::new(
            "SERVER_UNAUTHORIZED",
            "Server rejected the admin token (HTTP 401/403)",
        )
    }

    pub fn server_status(status: u16) -> Self {
        Self::new("SERVER_ERROR", format!("Server returned HTTP {status}"))
    }

    pub fn invalid_input(field: &str, detail: impl std::fmt::Display) -> Self {
        Self::new("INVALID_INPUT", format!("Invalid {field}")).with_detail(detail.to_string())
    }

    #[allow(dead_code)]
    pub fn sync_failed(detail: impl std::fmt::Display) -> Self {
        Self::new("SYNC_FAILED", "Sync failed").with_detail(detail.to_string())
    }

    pub fn internal(detail: impl std::fmt::Display) -> Self {
        Self::new("INTERNAL", "Internal error").with_detail(detail.to_string())
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(d) => write!(f, "{} ({})", self.message, d),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for AppError {}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(format!("{e:#}"))
    }
}

impl From<std::sync::mpsc::RecvTimeoutError> for AppError {
    fn from(_: std::sync::mpsc::RecvTimeoutError) -> Self {
        Self::device_timeout()
    }
}

/// Map a `reqwest::Error` into the most specific `AppError` we can. The
/// server API is bearer-authenticated, so a 401/403 is surfaced as
/// `SERVER_UNAUTHORIZED` rather than a generic unreachable.
pub fn from_reqwest(err: reqwest::Error) -> AppError {
    if let Some(status) = err.status() {
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return AppError::server_unauthorized();
        }
        return AppError::server_status(status.as_u16());
    }
    AppError::server_unreachable(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_as_camel_case_object() {
        let json = serde_json::to_string(&AppError::device_not_connected()).unwrap();
        assert!(json.contains("\"code\":\"DEVICE_NOT_CONNECTED\""));
        assert!(json.contains("\"message\":"));
        // detail is skipped when None
        assert!(!json.contains("\"detail\""));
    }

    #[test]
    fn carries_detail_when_present() {
        let err = AppError::usb_open_failed("device busy");
        assert_eq!(err.code, "USB_OPEN_FAILED");
        assert_eq!(err.detail.as_deref(), Some("device busy"));
    }

    #[test]
    fn identifies_wifi_scan_failures() {
        let err = AppError::wifi_scan_failed("turn on Wi-Fi and retry");
        assert_eq!(err.code, "WIFI_SCAN_FAILED");
        assert_eq!(err.message, "Wi-Fi scan unavailable");
        assert_eq!(err.detail.as_deref(), Some("turn on Wi-Fi and retry"));
    }
}
