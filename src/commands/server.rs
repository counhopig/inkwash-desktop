//! Tauri commands for talking to `inkwash-server`'s admin API. These
//! all run on a worker thread (`spawn_blocking`) since `reqwest` here
//! is in blocking mode (matches the existing `ServerClient`).
//!
//! The wire types come from `crate::server`. Error mapping goes
//! through `crate::error::from_reqwest` so 401/403 becomes
//! `SERVER_UNAUTHORIZED` rather than a generic unreachable.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::desktop::SharedState;
use crate::error::{from_reqwest, AppError};
use crate::server::{
    Alarm, Channel, ChannelCreated, Device, Importance, InboxItem, Repeat, ServerClient, Todo,
    TodoDue, UpsertAlarmRequest, UpsertTodoRequest,
};

fn client(base_url: String, token: String) -> Result<ServerClient, AppError> {
    let url = crate::commands::logs::normalise_server_url(&base_url)
        .ok_or_else(|| AppError::invalid_input("Server URL", "must not be empty"))?;
    Ok(ServerClient::new(url, token))
}

/// Shared shape of every admin command in this module: build a
/// [`ServerClient`] from the caller-supplied URL/token, run `f` on the
/// blocking thread pool (`reqwest` here is blocking - see the module docs),
/// then flatten the two error layers back into one `AppError`: a join
/// failure (worker panicked or the runtime shut down) becomes
/// `AppError::internal` tagged with `label`, while errors returned by `f`
/// go through [`map_err`] exactly once.
async fn admin_call<T, F>(
    label: &str,
    base_url: String,
    token: String,
    f: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&ServerClient) -> anyhow::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let c = client(base_url, token)?;
        f(&c).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("{label} task: {e}")))?
}

// ---------- Devices ----------

#[tauri::command]
pub async fn list_devices(
    base_url: String,
    token: String,
    state: State<'_, SharedState>,
) -> Result<Vec<Device>, AppError> {
    state
        .logs
        .info("server", format!("→ GET {base_url}/api/devices"));
    admin_call("list_devices", base_url, token, |c| c.list_devices()).await
}

#[tauri::command]
pub async fn register_device(
    base_url: String,
    token: String,
    name: String,
    state: State<'_, SharedState>,
) -> Result<Device, AppError> {
    state
        .logs
        .info("server", format!("→ POST {base_url}/api/devices name={name}"));
    tauri::async_runtime::spawn_blocking(move || -> Result<Device, AppError> {
        let c = client(base_url, token)?;
        c.register_device(&name).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("register_device task: {e}")))?
}

#[tauri::command]
pub async fn delete_device(
    base_url: String,
    token: String,
    device_id: String,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state
        .logs
        .info("server", format!("→ DELETE {base_url}/api/devices/{device_id}"));
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.delete_device(&device_id).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("delete_device task: {e}")))?
}

// ---------- Alarms ----------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmInput {
    pub hour: u8,
    pub minute: u8,
    pub label: String,
    pub repeat: Repeat,
    pub enabled: bool,
}

#[tauri::command]
pub async fn create_alarm(
    base_url: String,
    token: String,
    device_id: String,
    input: AlarmInput,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    validate_alarm(&input)?;
    state.logs.info(
        "server",
        format!(
            "→ POST {base_url}/api/devices/{device_id}/alarms {}:{:02}",
            input.hour, input.minute
        ),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        let req = UpsertAlarmRequest {
            hour: input.hour,
            minute: input.minute,
            label: input.label,
            repeat: input.repeat,
            enabled: input.enabled,
        };
        c.create_alarm(&device_id, &req).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("create_alarm task: {e}")))?
}

#[tauri::command]
pub async fn update_alarm(
    base_url: String,
    token: String,
    device_id: String,
    alarm_id: u8,
    input: AlarmInput,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    validate_alarm(&input)?;
    state.logs.info(
        "server",
        format!(
            "→ PUT {base_url}/api/devices/{device_id}/alarms/{alarm_id} {}:{:02}",
            input.hour, input.minute
        ),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        let req = UpsertAlarmRequest {
            hour: input.hour,
            minute: input.minute,
            label: input.label,
            repeat: input.repeat,
            enabled: input.enabled,
        };
        c.update_alarm(&device_id, alarm_id, &req)
            .map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("update_alarm task: {e}")))?
}

#[tauri::command]
pub async fn delete_alarm(
    base_url: String,
    token: String,
    device_id: String,
    alarm_id: u8,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state.logs.info(
        "server",
        format!("→ DELETE {base_url}/api/devices/{device_id}/alarms/{alarm_id}"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.delete_alarm(&device_id, alarm_id).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("delete_alarm task: {e}")))?
}

#[tauri::command]
pub async fn clear_alarms(
    base_url: String,
    token: String,
    device_id: String,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state.logs.warn(
        "server",
        format!("→ DELETE {base_url}/api/devices/{device_id}/alarms (clear)"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.clear_alarms(&device_id).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("clear_alarms task: {e}")))?
}

// ---------- Todos ----------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoInput {
    pub text: String,
    pub done: bool,
    #[serde(default)]
    pub importance: Importance,
    #[serde(default)]
    pub due_date: Option<TodoDue>,
    #[serde(default)]
    pub repeat: Option<Repeat>,
}

#[tauri::command]
pub async fn create_todo(
    base_url: String,
    token: String,
    device_id: String,
    input: TodoInput,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    validate_todo(&input)?;
    state.logs.info(
        "server",
        format!(
            "→ POST {base_url}/api/devices/{device_id}/todos len={}",
            input.text.chars().count()
        ),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        let req = UpsertTodoRequest {
            text: input.text,
            done: input.done,
            importance: input.importance,
            due_date: input.due_date,
            repeat: input.repeat,
        };
        c.create_todo(&device_id, &req).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("create_todo task: {e}")))?
}

#[tauri::command]
pub async fn update_todo(
    base_url: String,
    token: String,
    device_id: String,
    todo_id: u8,
    input: TodoInput,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    validate_todo(&input)?;
    state.logs.info(
        "server",
        format!(
            "→ PUT {base_url}/api/devices/{device_id}/todos/{todo_id} done={}",
            input.done
        ),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        let req = UpsertTodoRequest {
            text: input.text,
            done: input.done,
            importance: input.importance,
            due_date: input.due_date,
            repeat: input.repeat,
        };
        c.update_todo(&device_id, todo_id, &req).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("update_todo task: {e}")))?
}

#[tauri::command]
pub async fn delete_todo(
    base_url: String,
    token: String,
    device_id: String,
    todo_id: u8,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state.logs.info(
        "server",
        format!("→ DELETE {base_url}/api/devices/{device_id}/todos/{todo_id}"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.delete_todo(&device_id, todo_id).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("delete_todo task: {e}")))?
}

#[tauri::command]
pub async fn clear_todos(
    base_url: String,
    token: String,
    device_id: String,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state.logs.warn(
        "server",
        format!("→ DELETE {base_url}/api/devices/{device_id}/todos (clear)"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.clear_todos(&device_id).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("clear_todos task: {e}")))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSnapshot {
    pub alarms: Vec<Alarm>,
    pub todos: Vec<Todo>,
    pub channels: Vec<Channel>,
    pub inbox: Vec<InboxItem>,
}

#[tauri::command]
pub async fn list_content(
    base_url: String,
    token: String,
    device_id: String,
    state: State<'_, SharedState>,
) -> Result<ContentSnapshot, AppError> {
    state.logs.info(
        "server",
        format!("→ GET {base_url}/api/devices/{device_id}/(alarms+todos+channels+inbox)"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<ContentSnapshot, AppError> {
        let c = client(base_url, token)?;
        let alarms = c.list_alarms(&device_id).map_err(map_err)?;
        let todos = c.list_todos(&device_id).map_err(map_err)?;
        let channels = c.list_channels(&device_id).map_err(map_err)?;
        let inbox = c.list_inbox(&device_id).map_err(map_err)?;
        Ok(ContentSnapshot {
            alarms,
            todos,
            channels,
            inbox,
        })
    })
    .await
    .map_err(|e| AppError::internal(format!("list_content task: {e}")))?
}

// ---------- Channels & inbox ----------

#[tauri::command]
pub async fn create_webhook_channel(
    base_url: String,
    token: String,
    device_id: String,
    name: String,
    state: State<'_, SharedState>,
) -> Result<ChannelCreated, AppError> {
    if name.trim().is_empty() || name.chars().count() > 80 {
        return Err(AppError::invalid_input("name", "must be 1..80 chars"));
    }
    state.logs.info(
        "server",
        format!("→ POST {base_url}/api/devices/{device_id}/channels (webhook)"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<ChannelCreated, AppError> {
        let c = client(base_url, token)?;
        c.create_channel(&device_id, &name).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("create_webhook_channel task: {e}")))?
}

#[tauri::command]
pub async fn delete_channel(
    base_url: String,
    token: String,
    device_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state.logs.info(
        "server",
        format!("→ DELETE {base_url}/api/devices/{device_id}/channels/{channel_id}"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.delete_channel(&device_id, &channel_id).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("delete_channel task: {e}")))?
}

#[tauri::command]
pub async fn rotate_channel_token(
    base_url: String,
    token: String,
    device_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
) -> Result<String, AppError> {
    state.logs.info(
        "server",
        format!("→ POST {base_url}/api/devices/{device_id}/channels/{channel_id}/rotate-token"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<String, AppError> {
        let c = client(base_url, token)?;
        c.rotate_channel_token(&device_id, &channel_id)
            .map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("rotate_channel_token task: {e}")))?
}

#[tauri::command]
pub async fn delete_inbox_item(
    base_url: String,
    token: String,
    device_id: String,
    seq: u64,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state.logs.info(
        "server",
        format!("→ DELETE {base_url}/api/devices/{device_id}/inbox/{seq}"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.delete_inbox_item(&device_id, seq).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("delete_inbox_item task: {e}")))?
}

#[tauri::command]
pub async fn clear_inbox(
    base_url: String,
    token: String,
    device_id: String,
    state: State<'_, SharedState>,
) -> Result<(), AppError> {
    state.logs.warn(
        "server",
        format!("→ DELETE {base_url}/api/devices/{device_id}/inbox (clear read)"),
    );
    tauri::async_runtime::spawn_blocking(move || -> Result<(), AppError> {
        let c = client(base_url, token)?;
        c.clear_inbox(&device_id).map_err(map_err)
    })
    .await
    .map_err(|e| AppError::internal(format!("clear_inbox task: {e}")))?
}

// ---------- helpers ----------

fn map_err(e: anyhow::Error) -> AppError {
    match e.downcast::<reqwest::Error>() {
        Ok(req_err) => from_reqwest(req_err),
        Err(other) => AppError::server_unreachable(format!("{other:#}")),
    }
}

fn validate_alarm(input: &AlarmInput) -> Result<(), AppError> {
    if input.hour > 23 {
        return Err(AppError::invalid_input("hour", "must be 0..=23"));
    }
    if input.minute > 59 {
        return Err(AppError::invalid_input("minute", "must be 0..=59"));
    }
    if input.label.chars().count() > 32 {
        return Err(AppError::invalid_input("label", "longer than 32 chars"));
    }
    if let Repeat::Once { year, month, day } = input.repeat {
        if !(1900..=2200).contains(&year) {
            return Err(AppError::invalid_input("year", "out of range"));
        }
        if !(1..=12).contains(&month) {
            return Err(AppError::invalid_input("month", "must be 1..=12"));
        }
        if !(1..=31).contains(&day) {
            return Err(AppError::invalid_input("day", "must be 1..=31"));
        }
    }
    Ok(())
}

fn validate_todo(input: &TodoInput) -> Result<(), AppError> {
    let trimmed = input.text.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("text", "must not be empty"));
    }
    if input.text.chars().count() > 200 {
        return Err(AppError::invalid_input("text", "longer than 200 chars"));
    }
    Ok(())
}
