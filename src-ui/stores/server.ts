// Server-side admin API state: URL + admin token, registered devices,
// currently selected device, and the alarms/todos for that device.
// The token is loaded from the system keychain during bootstrap.

import { defineStore } from "pinia";
import { ref, computed } from "vue";
import * as C from "../lib/commands";
import {
  loadAdminToken,
  loadSelectedDeviceId,
  loadServerBaseUrl,
  saveAdminToken,
  saveSelectedDeviceId,
  saveServerBaseUrl,
} from "../lib/storage";
import type {
  Alarm,
  AlarmInput,
  Channel,
  ChannelCreated,
  Device,
  InboxItem,
  Todo,
  TodoInput,
} from "../lib/types";

export const useServerStore = defineStore("server", () => {
  const baseUrl = ref(loadServerBaseUrl());
  const adminToken = ref("");
  const connected = ref(false);
  const lastError = ref<{ code: string; message: string } | null>(null);

  const devices = ref<Device[]>([]);
  const selectedDeviceId = ref<string | null>(loadSelectedDeviceId());
  const alarms = ref<Alarm[]>([]);
  const todos = ref<Todo[]>([]);
  const channels = ref<Channel[]>([]);
  const inbox = ref<InboxItem[]>([]);

  function setBaseUrl(value: string) {
    baseUrl.value = value;
    saveServerBaseUrl(value);
    connected.value = false;
  }

  let tokenSaveQueue = Promise.resolve();

  function setAdminToken(value: string) {
    adminToken.value = value;
    connected.value = false;
    tokenSaveQueue = tokenSaveQueue
      .catch(() => undefined)
      .then(() => saveAdminToken(value))
      .catch((err: unknown) => {
        lastError.value = {
          code: "INTERNAL",
          message: err instanceof Error ? err.message : "Failed to save admin token",
        };
      });
  }

  async function bootstrap(): Promise<boolean> {
    try {
      adminToken.value = await loadAdminToken();
      return true;
    } catch (err) {
      lastError.value = {
        code: "INTERNAL",
        message: err instanceof Error ? err.message : "Failed to load admin token",
      };
      connected.value = false;
      return false;
    }
  }

  function selectDevice(id: string | null) {
    selectedDeviceId.value = id;
    saveSelectedDeviceId(id);
    alarms.value = [];
    todos.value = [];
    channels.value = [];
    inbox.value = [];
  }

  async function refreshDevices() {
    if (!baseUrl.value.trim()) {
      connected.value = false;
      return { ok: false as const, error: { code: "INVALID_INPUT", message: "No server URL configured" } };
    }
    const r = await C.listDevices(baseUrl.value, adminToken.value);
    if (r.ok) {
      devices.value = r.value;
      connected.value = true;
      lastError.value = null;
      // A device selected in an earlier session (persisted to localStorage)
      // may no longer exist on this server - e.g. its DB was reset, or the
      // base URL now points at a different server. Drop the stale
      // selection instead of leaving it referencing a nonexistent device,
      // which would otherwise let alarm/todo writes hit a foreign key
      // that no longer resolves.
      if (
        selectedDeviceId.value != null &&
        !r.value.some((d) => d.id === selectedDeviceId.value)
      ) {
        selectDevice(null);
      }
    } else {
      lastError.value = { code: r.error.code, message: r.error.message };
      connected.value = false;
    }
    return r;
  }

  async function registerDevice(name: string) {
    const r = await C.registerDevice(baseUrl.value, adminToken.value, name);
    if (r.ok) {
      devices.value = [...devices.value, r.value];
    } else {
      lastError.value = { code: r.error.code, message: r.error.message };
    }
    return r;
  }

  async function deleteDevice(id: string) {
    const r = await C.deleteDevice(baseUrl.value, adminToken.value, id);
    if (r.ok) {
      devices.value = devices.value.filter((d) => d.id !== id);
      if (selectedDeviceId.value === id) selectDevice(null);
    } else {
      lastError.value = { code: r.error.code, message: r.error.message };
    }
    return r;
  }

  async function refreshContent() {
    if (selectedDeviceId.value == null) return;
    const r = await C.listContent(baseUrl.value, adminToken.value, selectedDeviceId.value);
    if (r.ok) {
      alarms.value = r.value.alarms;
      todos.value = r.value.todos;
      channels.value = r.value.channels;
      inbox.value = r.value.inbox;
      connected.value = true;
      lastError.value = null;
    } else {
      lastError.value = { code: r.error.code, message: r.error.message };
      connected.value = false;
    }
    return r;
  }

  // Every device-scoped write below shares the same shape: bail out if no
  // device is selected, call the server, refresh content on success or
  // record the error on failure. Centralising it here means a change to
  // that shape (e.g. optimistic updates instead of a full refetch) only
  // has to happen once.
  function withSelectedDevice<T>(
    action: (deviceId: string) => Promise<C.Result<T>>,
  ): Promise<C.Result<T>> {
    if (selectedDeviceId.value == null) {
      return Promise.resolve({
        ok: false,
        error: { code: "INVALID_INPUT", message: "No device selected" },
      });
    }
    return action(selectedDeviceId.value).then(async (r) => {
      if (r.ok) await refreshContent();
      else lastError.value = { code: r.error.code, message: r.error.message };
      return r;
    });
  }

  function createAlarm(input: AlarmInput) {
    return withSelectedDevice((deviceId) => C.createAlarm(baseUrl.value, adminToken.value, deviceId, input));
  }

  function updateAlarm(id: number, input: AlarmInput) {
    return withSelectedDevice((deviceId) => C.updateAlarm(baseUrl.value, adminToken.value, deviceId, id, input));
  }

  function deleteAlarm(id: number) {
    return withSelectedDevice((deviceId) => C.deleteAlarm(baseUrl.value, adminToken.value, deviceId, id));
  }

  function clearAlarms() {
    return withSelectedDevice((deviceId) => C.clearAlarms(baseUrl.value, adminToken.value, deviceId));
  }

  function createTodo(input: TodoInput) {
    return withSelectedDevice((deviceId) => C.createTodo(baseUrl.value, adminToken.value, deviceId, input));
  }

  function updateTodo(id: number, input: TodoInput) {
    return withSelectedDevice((deviceId) => C.updateTodo(baseUrl.value, adminToken.value, deviceId, id, input));
  }

  function deleteTodo(id: number) {
    return withSelectedDevice((deviceId) => C.deleteTodo(baseUrl.value, adminToken.value, deviceId, id));
  }

  function clearTodos() {
    return withSelectedDevice((deviceId) => C.clearTodos(baseUrl.value, adminToken.value, deviceId));
  }

  function createWebhookChannel(name: string): Promise<C.Result<ChannelCreated>> {
    return withSelectedDevice((deviceId) => C.createWebhookChannel(baseUrl.value, adminToken.value, deviceId, name));
  }

  function deleteChannel(channelId: string) {
    return withSelectedDevice((deviceId) => C.deleteChannel(baseUrl.value, adminToken.value, deviceId, channelId));
  }

  function rotateChannelToken(channelId: string) {
    return withSelectedDevice((deviceId) => C.rotateChannelToken(baseUrl.value, adminToken.value, deviceId, channelId));
  }

  function deleteInboxItem(seq: number) {
    return withSelectedDevice((deviceId) => C.deleteInboxItem(baseUrl.value, adminToken.value, deviceId, seq));
  }

  function clearInbox() {
    return withSelectedDevice((deviceId) => C.clearInbox(baseUrl.value, adminToken.value, deviceId));
  }

  const selectedDevice = computed(() =>
    devices.value.find((d) => d.id === selectedDeviceId.value) ?? null,
  );

  const alarmCount = computed(() => alarms.value.length);
  const todoCount = computed(() => todos.value.length);
  const todoDoneCount = computed(() => todos.value.filter((t) => t.done).length);

  return {
    baseUrl,
    adminToken,
    connected,
    lastError,
    devices,
    selectedDeviceId,
    selectedDevice,
    alarms,
    todos,
    channels,
    inbox,
    alarmCount,
    todoCount,
    todoDoneCount,
    setBaseUrl,
    setAdminToken,
    bootstrap,
    selectDevice,
    refreshDevices,
    registerDevice,
    deleteDevice,
    refreshContent,
    createAlarm,
    updateAlarm,
    deleteAlarm,
    clearAlarms,
    createTodo,
    updateTodo,
    deleteTodo,
    clearTodos,
    createWebhookChannel,
    deleteChannel,
    rotateChannelToken,
    deleteInboxItem,
    clearInbox,
  };
});
