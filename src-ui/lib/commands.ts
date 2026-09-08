// Thin wrappers around `invoke()` from @tauri-apps/api that surface a
// strongly-typed `Result<T, AppError>` instead of throwing. The Rust
// commands already return `Result<T, AppError>`, so the catch block
// converts the rejected promise into an `AppError`.

import { invoke } from "@tauri-apps/api/core";
import type {
  AlarmInput,
  AppError,
  ChannelCreated,
  ConnectionStateInfo,
  ContentSnapshot,
  Device,
  DeviceCommandResult,
  LogEntry,
  TodoInput,
  WifiNetwork,
} from "./types";

export type Result<T> = { ok: true; value: T } | { ok: false; error: AppError };

function wrap<T>(p: Promise<T>): Promise<Result<T>> {
  return p.then(
    (value) => ({ ok: true as const, value }),
    (err) => ({ ok: false as const, error: normaliseError(err) }),
  );
}

function normaliseError(raw: unknown): AppError {
  if (raw && typeof raw === "object" && "code" in (raw as Record<string, unknown>)) {
    return raw as AppError;
  }
  if (typeof raw === "string") {
    return { code: "INTERNAL", message: raw };
  }
  return {
    code: "INTERNAL",
    message: "Unexpected error",
    detail: typeof raw === "object" ? JSON.stringify(raw) : String(raw),
  };
}

export async function listUsbPorts(): Promise<Result<string[]>> {
  return wrap(invoke<string[]>("list_usb_ports"));
}

export async function connectUsb(port: string): Promise<Result<null>> {
  return wrap(invoke<null>("connect_usb", { port }));
}

export async function disconnectDevice(): Promise<Result<null>> {
  return wrap(invoke<null>("disconnect_device"));
}

export async function discoverBle(): Promise<Result<boolean>> {
  return wrap(invoke<boolean>("discover_ble"));
}

export async function connectBle(): Promise<Result<null>> {
  return wrap(invoke<null>("connect_ble"));
}

export async function getConnectionState(): Promise<Result<ConnectionStateInfo>> {
  return wrap(invoke<ConnectionStateInfo>("get_connection_state"));
}

export async function getDeviceStatus(): Promise<Result<DeviceCommandResult>> {
  return wrap(invoke<DeviceCommandResult>("get_device_status"));
}

export async function setWifi(ssid: string, password: string): Promise<Result<DeviceCommandResult>> {
  return wrap(invoke<DeviceCommandResult>("set_wifi", { ssid, password }));
}

export async function setServer(url: string, token: string): Promise<Result<DeviceCommandResult>> {
  return wrap(invoke<DeviceCommandResult>("set_server", { url, token }));
}

export async function setTimezone(offsetMinutes: number): Promise<Result<DeviceCommandResult>> {
  return wrap(invoke<DeviceCommandResult>("set_timezone", { offsetMinutes }));
}

export async function syncNow(): Promise<Result<DeviceCommandResult>> {
  return wrap(invoke<DeviceCommandResult>("sync_now"));
}

export async function clearDeviceAlarms(): Promise<Result<DeviceCommandResult>> {
  return wrap(invoke<DeviceCommandResult>("clear_device_alarms"));
}

// ---------- Wi-Fi scan (PC-side) ----------

export async function scanWifiNetworks(): Promise<Result<WifiNetwork[]>> {
  return wrap(invoke<WifiNetwork[]>("scan_wifi_networks"));
}

// ---------- Server ----------

export async function listDevices(baseUrl: string, token: string): Promise<Result<Device[]>> {
  return wrap(invoke<Device[]>("list_devices", { baseUrl, token }));
}

export async function registerDevice(baseUrl: string, token: string, name: string): Promise<Result<Device>> {
  return wrap(invoke<Device>("register_device", { baseUrl, token, name }));
}

export async function deleteDevice(baseUrl: string, token: string, deviceId: string): Promise<Result<null>> {
  return wrap(invoke<null>("delete_device", { baseUrl, token, deviceId }));
}

export async function createAlarm(
  baseUrl: string,
  token: string,
  deviceId: string,
  input: AlarmInput,
): Promise<Result<null>> {
  return wrap(invoke<null>("create_alarm", { baseUrl, token, deviceId, input }));
}

export async function updateAlarm(
  baseUrl: string,
  token: string,
  deviceId: string,
  alarmId: number,
  input: AlarmInput,
): Promise<Result<null>> {
  return wrap(invoke<null>("update_alarm", { baseUrl, token, deviceId, alarmId, input }));
}

export async function deleteAlarm(
  baseUrl: string,
  token: string,
  deviceId: string,
  alarmId: number,
): Promise<Result<null>> {
  return wrap(invoke<null>("delete_alarm", { baseUrl, token, deviceId, alarmId }));
}

export async function clearAlarms(baseUrl: string, token: string, deviceId: string): Promise<Result<null>> {
  return wrap(invoke<null>("clear_alarms", { baseUrl, token, deviceId }));
}

export async function createTodo(
  baseUrl: string,
  token: string,
  deviceId: string,
  input: TodoInput,
): Promise<Result<null>> {
  return wrap(invoke<null>("create_todo", { baseUrl, token, deviceId, input }));
}

export async function updateTodo(
  baseUrl: string,
  token: string,
  deviceId: string,
  todoId: number,
  input: TodoInput,
): Promise<Result<null>> {
  return wrap(invoke<null>("update_todo", { baseUrl, token, deviceId, todoId, input }));
}

export async function deleteTodo(
  baseUrl: string,
  token: string,
  deviceId: string,
  todoId: number,
): Promise<Result<null>> {
  return wrap(invoke<null>("delete_todo", { baseUrl, token, deviceId, todoId }));
}

export async function clearTodos(baseUrl: string, token: string, deviceId: string): Promise<Result<null>> {
  return wrap(invoke<null>("clear_todos", { baseUrl, token, deviceId }));
}

export async function listContent(baseUrl: string, token: string, deviceId: string): Promise<Result<ContentSnapshot>> {
  return wrap(invoke<ContentSnapshot>("list_content", { baseUrl, token, deviceId }));
}

export async function createWebhookChannel(
  baseUrl: string,
  token: string,
  deviceId: string,
  name: string,
): Promise<Result<ChannelCreated>> {
  return wrap(invoke<ChannelCreated>("create_webhook_channel", { baseUrl, token, deviceId, name }));
}

export async function deleteChannel(
  baseUrl: string,
  token: string,
  deviceId: string,
  channelId: string,
): Promise<Result<null>> {
  return wrap(invoke<null>("delete_channel", { baseUrl, token, deviceId, channelId }));
}

export async function rotateChannelToken(
  baseUrl: string,
  token: string,
  deviceId: string,
  channelId: string,
): Promise<Result<string>> {
  return wrap(invoke<string>("rotate_channel_token", { baseUrl, token, deviceId, channelId }));
}

export async function deleteInboxItem(
  baseUrl: string,
  token: string,
  deviceId: string,
  seq: number,
): Promise<Result<null>> {
  return wrap(invoke<null>("delete_inbox_item", { baseUrl, token, deviceId, seq }));
}

export async function clearInbox(baseUrl: string, token: string, deviceId: string): Promise<Result<null>> {
  return wrap(invoke<null>("clear_inbox", { baseUrl, token, deviceId }));
}

export async function loadAdminToken(): Promise<Result<string>> {
  return wrap(invoke<string>("load_admin_token"));
}

export async function saveAdminToken(token: string): Promise<Result<null>> {
  return wrap(invoke<null>("save_admin_token", { token }));
}

// ---------- Logs ----------

export async function readLogs(): Promise<Result<LogEntry[]>> {
  return wrap(invoke<LogEntry[]>("read_logs"));
}

export async function clearLogs(): Promise<Result<null>> {
  return wrap(invoke<null>("clear_logs"));
}

export async function logFilePath(): Promise<Result<string>> {
  return wrap(invoke<string>("log_file_path"));
}

export async function logDir(): Promise<Result<string>> {
  return wrap(invoke<string>("log_dir"));
}

export async function openLogFolder(): Promise<Result<null>> {
  return wrap(invoke<null>("open_log_folder"));
}

export async function exportLog(): Promise<Result<string>> {
  return wrap(invoke<string>("export_log"));
}
