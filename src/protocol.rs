//! Wire types mirroring the firmware's `control.rs`
//! (`inkwash/docs/control-protocol.md`) - this crate sends `Command` JSON
//! and parses `Reply` JSON, so the tag/field names must match the
//! firmware's `Serialize`/`Deserialize` derives exactly. USB and BLE both
//! carry the same JSON payloads; only the framing differs (see
//! `transport::usb` vs `transport::ble`).

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Generates a fresh, process-unique request correlation id (see
/// `encode_command`). Callers should generate one per logical request and
/// reuse it across any resends of that same request, not mint a new one per
/// resend attempt - a delayed reply to an earlier resend must still match.
pub fn next_request_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    format!("req-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    SetWifi { ssid: String, password: String },
    SetServer { url: String, token: String },
    SyncNow,
    SetRtc { epoch_secs: u64 },
    GetStatus,
    ClearAlarms,
    SetTimezone { offset_minutes: i16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    /// The device was showing a full-screen reminder (due-todo or urgent
    /// inbox) and did not execute the command - see `control-protocol.md`'s
    /// `Busy` reply. Not executed, safe to resend.
    Busy,
    Status {
        wifi_configured: bool,
        server_configured: bool,
        wifi_connected: bool,
        #[serde(default)]
        wifi_ssid: Option<String>,
        #[serde(default)]
        wifi_has_password: bool,
        #[serde(default)]
        server_url: Option<String>,
        #[serde(default)]
        server_has_token: bool,
        #[serde(default)]
        timezone_offset_minutes: i16,
    },
    Error {
        message: String,
    },
}

/// Encodes `cmd` as JSON with a top-level `id` field (any string) added so
/// the reply can be matched back to this specific request - see
/// `control-protocol.md`'s "Request Correlation" section. The firmware
/// echoes `id` back on the reply unchanged; older firmware without this
/// feature just ignores the extra field.
pub fn encode_command(cmd: &Command, id: &str) -> String {
    // Serialization is total: `serde_json::to_value` can only fail on a
    // `Serialize` impl that returns an error itself - either a map with
    // non-string keys, data `#[serde(flatten)]`ed into a non-self-describing
    // format, or hand-written `serialize_*` calls that propagate `Err`. The
    // `Command` enum above is derived and its fields are exactly `String`,
    // `i16`, and unit variants, so no such step exists and this cannot
    // panic in practice.
    let mut value = serde_json::to_value(cmd).expect("Command always serializes");
    if let Some(obj) = value.as_object_mut() {
        obj.insert("id".to_string(), serde_json::Value::String(id.to_string()));
    }
    value.to_string()
}

/// Decodes a reply line, returning its correlation `id` alongside it.
/// `id` is `None` when the device didn't echo one back - either the
/// triggering command had none, or (for `main.rs`'s CLI, which doesn't set
/// one) always. Callers that care about matching a specific in-flight
/// request should treat `None` as "can't verify, accept on trust" rather
/// than a mismatch, since older firmware never sends `id` at all.
pub fn decode_reply(line: &str) -> anyhow::Result<(Option<String>, Reply)> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|e| anyhow::anyhow!("failed to parse reply '{line}': {e}"))?;
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let reply = serde_json::from_value(value)
        .map_err(|e| anyhow::anyhow!("failed to parse reply '{line}': {e}"))?;
    Ok((id, reply))
}

/// What to do with a reply that just arrived, relative to the request
/// currently in flight. Shared by the CLI's retry loop (`main.rs`) and the
/// Tauri command retry loop (`commands/device.rs`) - they run over two
/// differently-typed transports (a bare `mpsc::Receiver` vs a
/// mutex-guarded `AppState`) so the surrounding loops can't be unified
/// outright, but the actual "is this reply mine, and if not what do I do"
/// decision is exactly the same logic and shouldn't be hand-written twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyDecision {
    /// `reply_id` doesn't match ours; it belongs to some other in-flight or
    /// stale request and must not be treated as our answer.
    Stale,
    /// The device was busy (a full-screen reminder was showing) and did
    /// not execute the command; resend and keep waiting.
    Busy,
    /// This is the answer to our request.
    Done,
}

/// Classifies `reply` against `request_id`. A reply with no `id` at all is
/// accepted on trust rather than treated as stale, since firmware that
/// predates request correlation never sends one - see `decode_reply`'s doc
/// comment.
pub fn classify_reply(request_id: &str, reply_id: Option<&str>, reply: &Reply) -> ReplyDecision {
    if let Some(rid) = reply_id {
        if rid != request_id {
            return ReplyDecision::Stale;
        }
    }
    if matches!(reply, Reply::Busy) {
        return ReplyDecision::Busy;
    }
    ReplyDecision::Done
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses `json` back and checks it has exactly the expected top-level
    /// keys/values, independent of the field order `serde_json::Value`
    /// happens to serialize in.
    fn assert_json_object_eq(json: &str, expected: &[(&str, serde_json::Value)]) {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let obj = value.as_object().expect("expected a JSON object");
        assert_eq!(obj.len(), expected.len(), "field count mismatch in {json}");
        for (key, expected_value) in expected {
            assert_eq!(
                obj.get(*key),
                Some(expected_value),
                "field '{key}' in {json}"
            );
        }
    }

    #[test]
    fn firmware_commands_use_expected_wire_names() {
        assert_json_object_eq(
            &encode_command(&Command::GetStatus, "1"),
            &[("cmd", "get_status".into()), ("id", "1".into())],
        );
        assert_json_object_eq(
            &encode_command(&Command::ClearAlarms, "2"),
            &[("cmd", "clear_alarms".into()), ("id", "2".into())],
        );
        assert_json_object_eq(
            &encode_command(&Command::SetRtc { epoch_secs: 1_756_000_000 }, "2b"),
            &[
                ("cmd", "set_rtc".into()),
                ("epoch_secs", 1_756_000_000u64.into()),
                ("id", "2b".into()),
            ],
        );
        assert_json_object_eq(
            &encode_command(
                &Command::SetTimezone {
                    offset_minutes: 480,
                },
                "3",
            ),
            &[
                ("cmd", "set_timezone".into()),
                ("id", "3".into()),
                ("offset_minutes", 480.into()),
            ],
        );
    }

    #[test]
    fn request_ids_are_unique_and_ordered() {
        let a = next_request_id();
        let b = next_request_id();
        assert_ne!(a, b);
    }

    #[test]
    fn decodes_reply_id_when_present() {
        let (id, reply) = decode_reply(r#"{"status":"ok","id":"req-42"}"#).unwrap();
        assert_eq!(id.as_deref(), Some("req-42"));
        assert!(matches!(reply, Reply::Ok));
    }

    #[test]
    fn decodes_reply_with_no_id_as_none() {
        let (id, reply) = decode_reply(r#"{"status":"ok"}"#).unwrap();
        assert_eq!(id, None);
        assert!(matches!(reply, Reply::Ok));
    }

    #[test]
    fn decodes_busy_reply() {
        let (_id, reply) = decode_reply(r#"{"status":"busy"}"#).unwrap();
        assert!(matches!(reply, Reply::Busy));
    }

    #[test]
    fn decodes_status_reply() {
        let (_id, reply) = decode_reply(
            r#"{"status":"status","wifi_configured":true,"server_configured":false,"wifi_connected":true,"wifi_ssid":"MyNet","wifi_has_password":true,"server_url":"http://192.168.1.10:8080/api/sync","server_has_token":true,"timezone_offset_minutes":480}"#,
        )
        .unwrap();
        match reply {
            Reply::Status {
                wifi_configured,
                server_configured,
                wifi_connected,
                wifi_ssid,
                wifi_has_password,
                server_url,
                server_has_token,
                timezone_offset_minutes,
            } => {
                assert!(wifi_configured);
                assert!(!server_configured);
                assert!(wifi_connected);
                assert_eq!(wifi_ssid.as_deref(), Some("MyNet"));
                assert!(wifi_has_password);
                assert_eq!(
                    server_url.as_deref(),
                    Some("http://192.168.1.10:8080/api/sync")
                );
                assert!(server_has_token);
                assert_eq!(timezone_offset_minutes, 480);
            }
            _ => panic!("expected Status reply"),
        }
    }

    #[test]
    fn classify_reply_flags_a_mismatched_id_as_stale() {
        assert_eq!(
            classify_reply("req-2", Some("req-1"), &Reply::Ok),
            ReplyDecision::Stale
        );
    }

    #[test]
    fn classify_reply_accepts_a_missing_id_on_trust() {
        assert_eq!(
            classify_reply("req-2", None, &Reply::Ok),
            ReplyDecision::Done
        );
    }

    #[test]
    fn classify_reply_flags_busy_even_with_a_matching_id() {
        assert_eq!(
            classify_reply("req-2", Some("req-2"), &Reply::Busy),
            ReplyDecision::Busy
        );
    }

    #[test]
    fn classify_reply_is_done_for_a_matching_non_busy_reply() {
        assert_eq!(
            classify_reply("req-2", Some("req-2"), &Reply::Ok),
            ReplyDecision::Done
        );
    }

    #[test]
    fn decodes_legacy_status_reply_without_new_fields() {
        let (_id, reply) = decode_reply(
            r#"{"status":"status","wifi_configured":true,"server_configured":false,"wifi_connected":false}"#,
        )
        .unwrap();
        match reply {
            Reply::Status {
                wifi_ssid,
                wifi_has_password,
                server_url,
                server_has_token,
                timezone_offset_minutes,
                ..
            } => {
                assert!(wifi_ssid.is_none());
                assert!(!wifi_has_password);
                assert!(server_url.is_none());
                assert!(!server_has_token);
                assert_eq!(timezone_offset_minutes, 0);
            }
            _ => panic!("expected Status reply"),
        }
    }
}
