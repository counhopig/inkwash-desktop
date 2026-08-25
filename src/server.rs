//! HTTP client for `inkwash-server`'s admin API (device registration,
//! alarm/todo management). Mirrors the wire types in
//! `inkwash-server/src/models.rs`, which in turn mirror the firmware's
//! `alarms::StoredAlarm`/`todos::Todo` - three independent copies of the
//! same shape across three repos/languages-in-spirit, kept honest by the
//! shared JSON contract rather than shared code.
//!
//! Every call here is a blocking `reqwest` request; callers (the Tauri
//! commands) must run these on a background thread via
//! `tauri::async_runtime::spawn_blocking`, not directly in the command
//! body, or the UI thread will block for the duration of the request.

use serde::{Deserialize, Serialize};

/// Recurrence schedule, mirroring `inkwash-server`'s `models::Repeat`.
/// Externally tagged: `"Daily"`, `{"Weekly": {"days": [...]}}`,
/// `{"Monthly": {"days": [...]}}`, or `{"Once": {...}}`. Weekdays are
/// 0=Sunday..6=Saturday; month days are 1..=31.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Repeat {
    Daily,
    Weekly { days: Vec<u8> },
    Monthly { days: Vec<u8> },
    Once { year: u16, month: u8, day: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alarm {
    pub id: u8,
    pub hour: u8,
    pub minute: u8,
    pub repeat: Repeat,
    pub enabled: bool,
    pub label: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TodoDue {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Todo {
    pub id: u8,
    pub text: String,
    pub done: bool,
    #[serde(default)]
    pub importance: Importance,
    #[serde(default)]
    pub due_date: Option<TodoDue>,
    #[serde(default)]
    pub repeat: Option<Repeat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// UUID string, opaque - not a sequential number.
    pub id: String,
    pub name: String,
    pub token: Option<String>,
}

/// External channel (webhook / CalDAV) bound to a device - mirrors
/// `inkwash-server`'s `models::Channel`. Never contains the plaintext token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub device_id: String,
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    pub token_prefix: String,
    pub last_sync_at: Option<i64>,
    pub last_sync_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Response to creating a webhook channel: the plaintext token is returned
/// exactly once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelCreated {
    pub channel: Channel,
    pub token: Option<String>,
    pub delivery_url: Option<String>,
}

/// Inbox notification as seen over the admin API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: u64,
    pub kind: String,
    /// `"normal"` | `"high"` - the server always sends this (see
    /// `inkwash-server/src/models.rs`'s `InboxItem`), so this has no
    /// `#[serde(default)]` like `body`/`when`/`read` below. Previously
    /// missing here entirely, which meant `lib/types.ts`'s `InboxItem.
    /// priority` (a required, non-optional field) was silently `undefined`
    /// at runtime for every item.
    pub priority: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub when: Option<i64>,
    #[serde(default)]
    pub read: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpsertAlarmRequest {
    pub hour: u8,
    pub minute: u8,
    pub repeat: Repeat,
    pub enabled: bool,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpsertTodoRequest {
    pub text: String,
    pub done: bool,
    #[serde(default)]
    pub importance: Importance,
    #[serde(default)]
    pub due_date: Option<TodoDue>,
    #[serde(default)]
    pub repeat: Option<Repeat>,
}

#[derive(Clone)]
pub struct ServerClient {
    base_url: String,
    admin_token: String,
    client: reqwest::blocking::Client,
}

impl ServerClient {
    pub fn new(base_url: String, admin_token: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            admin_token,
            client: reqwest::blocking::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn auth(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        builder.bearer_auth(&self.admin_token)
    }

    pub fn register_device(&self, name: &str) -> anyhow::Result<Device> {
        let resp = self
            .auth(self.client.post(self.url("/api/devices")))
            .json(&serde_json::json!({ "name": name }))
            .send()?
            .error_for_status()?;
        Ok(resp.json()?)
    }

    pub fn list_devices(&self) -> anyhow::Result<Vec<Device>> {
        let resp = self
            .auth(self.client.get(self.url("/api/devices")))
            .send()?
            .error_for_status()?;
        Ok(resp.json()?)
    }

    pub fn delete_device(&self, id: &str) -> anyhow::Result<()> {
        self.auth(self.client.delete(self.url(&format!("/api/devices/{id}"))))
            .send()?
            .error_for_status()?;
        Ok(())
    }

    pub fn list_alarms(&self, device_id: &str) -> anyhow::Result<Vec<Alarm>> {
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/api/devices/{device_id}/alarms"))),
            )
            .send()?
            .error_for_status()?;
        Ok(resp.json()?)
    }

    pub fn create_alarm(&self, device_id: &str, req: &UpsertAlarmRequest) -> anyhow::Result<()> {
        self.auth(
            self.client
                .post(self.url(&format!("/api/devices/{device_id}/alarms"))),
        )
        .json(req)
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn update_alarm(
        &self,
        device_id: &str,
        alarm_id: u8,
        req: &UpsertAlarmRequest,
    ) -> anyhow::Result<()> {
        self.auth(
            self.client
                .put(self.url(&format!("/api/devices/{device_id}/alarms/{alarm_id}"))),
        )
        .json(req)
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn delete_alarm(&self, device_id: &str, alarm_id: u8) -> anyhow::Result<()> {
        self.auth(
            self.client
                .delete(self.url(&format!("/api/devices/{device_id}/alarms/{alarm_id}"))),
        )
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn clear_alarms(&self, device_id: &str) -> anyhow::Result<()> {
        self.auth(
            self.client
                .delete(self.url(&format!("/api/devices/{device_id}/alarms"))),
        )
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn list_todos(&self, device_id: &str) -> anyhow::Result<Vec<Todo>> {
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/api/devices/{device_id}/todos"))),
            )
            .send()?
            .error_for_status()?;
        Ok(resp.json()?)
    }

    pub fn create_todo(&self, device_id: &str, req: &UpsertTodoRequest) -> anyhow::Result<()> {
        self.auth(
            self.client
                .post(self.url(&format!("/api/devices/{device_id}/todos"))),
        )
        .json(req)
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn update_todo(
        &self,
        device_id: &str,
        todo_id: u8,
        req: &UpsertTodoRequest,
    ) -> anyhow::Result<()> {
        self.auth(
            self.client
                .put(self.url(&format!("/api/devices/{device_id}/todos/{todo_id}"))),
        )
        .json(req)
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn delete_todo(&self, device_id: &str, todo_id: u8) -> anyhow::Result<()> {
        self.auth(
            self.client
                .delete(self.url(&format!("/api/devices/{device_id}/todos/{todo_id}"))),
        )
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn clear_todos(&self, device_id: &str) -> anyhow::Result<()> {
        self.auth(
            self.client
                .delete(self.url(&format!("/api/devices/{device_id}/todos"))),
        )
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn list_channels(&self, device_id: &str) -> anyhow::Result<Vec<Channel>> {
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/api/devices/{device_id}/channels"))),
            )
            .send()?
            .error_for_status()?;
        Ok(resp.json()?)
    }

    pub fn create_channel(&self, device_id: &str, name: &str) -> anyhow::Result<ChannelCreated> {
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!("/api/devices/{device_id}/channels"))),
            )
            .json(&serde_json::json!({ "kind": "webhook", "name": name }))
            .send()?
            .error_for_status()?;
        Ok(resp.json()?)
    }

    pub fn delete_channel(&self, device_id: &str, channel_id: &str) -> anyhow::Result<()> {
        self.auth(
            self.client
                .delete(self.url(&format!(
                    "/api/devices/{device_id}/channels/{channel_id}"
                ))),
        )
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn rotate_channel_token(
        &self,
        device_id: &str,
        channel_id: &str,
    ) -> anyhow::Result<String> {
        let resp = self
            .auth(
                self.client
                    .post(self.url(&format!(
                        "/api/devices/{device_id}/channels/{channel_id}/rotate-token"
                    ))),
            )
            .send()?
            .error_for_status()?;
        let v: serde_json::Value = resp.json()?;
        Ok(v["token"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub fn list_inbox(&self, device_id: &str) -> anyhow::Result<Vec<InboxItem>> {
        let resp = self
            .auth(
                self.client
                    .get(self.url(&format!("/api/devices/{device_id}/inbox"))),
            )
            .send()?
            .error_for_status()?;
        Ok(resp.json()?)
    }

    pub fn delete_inbox_item(&self, device_id: &str, seq: u64) -> anyhow::Result<()> {
        self.auth(
            self.client
                .delete(self.url(&format!("/api/devices/{device_id}/inbox/{seq}"))),
        )
        .send()?
        .error_for_status()?;
        Ok(())
    }

    pub fn clear_inbox(&self, device_id: &str) -> anyhow::Result<()> {
        self.auth(
            self.client
                .delete(self.url(&format!("/api/devices/{device_id}/inbox"))),
        )
        .send()?
        .error_for_status()?;
        Ok(())
    }
}

/// Live integration tests against a real `inkwash-server` instance - they
/// are skipped by default because they hardcode a LAN address (`LIVE_URL`
/// below) that only exists on the home network. To run them manually,
/// have that server reachable and use:
///
/// ```text
/// cargo test -- --ignored
/// ```
#[cfg(test)]
mod live_server_tests {
    use super::*;
    use std::time::Duration;

    const LIVE_URL: &str = "http://192.168.1.10:8080";

    fn reachable() -> bool {
        let probe = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("client");
        probe
            .get(LIVE_URL)
            .send()
            .map(|r| r.status().is_client_error() || r.status().is_success())
            .unwrap_or(false)
    }

    #[test]
    #[ignore = "needs the live server on the LAN (hardcoded LIVE_URL); run manually with `cargo test -- --ignored`"]
    fn unauthenticated_list_devices_maps_to_unauthorized() {
        if !reachable() {
            eprintln!("[skip] {LIVE_URL} not reachable");
            return;
        }
        let c = ServerClient::new(LIVE_URL.into(), "dummy".into());
        let err = c.list_devices().expect_err("expected auth error");
        let downcasted = err.downcast::<reqwest::Error>().expect("reqwest error");
        let status = downcasted.status().expect("status code");
        assert!(
            status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN,
            "expected 401/403, got {status}"
        );
    }

    #[test]
    #[ignore = "needs the live server on the LAN (hardcoded LIVE_URL); run manually with `cargo test -- --ignored`"]
    fn unauthenticated_register_device_maps_to_unauthorized() {
        if !reachable() {
            eprintln!("[skip] {LIVE_URL} not reachable");
            return;
        }
        let c = ServerClient::new(LIVE_URL.into(), "dummy".into());
        let err = c.register_device("inkwash-cli-test").expect_err("expected auth error");
        let downcasted = err.downcast::<reqwest::Error>().expect("reqwest error");
        let status = downcasted.status().expect("status code");
        assert!(
            status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN,
            "expected 401/403, got {status}"
        );
    }
}
