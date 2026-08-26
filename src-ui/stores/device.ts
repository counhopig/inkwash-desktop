// Device connection state and command results. Held in a Pinia store
// so all pages (Overview, Device) can read the same connection info
// without prop-drilling.

import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { listen } from "@tauri-apps/api/event";
import * as C from "../lib/commands";
import type {
  ConnectionStateInfo,
  DeviceCommandResult,
  DeviceStatus,
} from "../lib/types";

export type OperationState = "idle" | "running" | "success" | "error";

interface Operation {
  state: OperationState;
  errorCode?: string;
  errorMessage?: string;
}

export const useDeviceStore = defineStore("device", () => {
  const connection = ref<ConnectionStateInfo>({
    connected: false,
    kind: "Offline",
    port: "—",
  });
  const usbPorts = ref<string[]>([]);
  const deviceStatus = ref<DeviceStatus | null>(null);
  const lastReplyAt = ref<number | null>(null);
  const lastResult = ref<DeviceCommandResult | null>(null);
  const ops = ref<Record<string, Operation>>({});

  async function bootstrap() {
    // Initial state from Rust.
    const cs = await C.getConnectionState();
    if (cs.ok) connection.value = cs.value;
    await refreshPorts();

    // Subscribe to real-time events. Failing to subscribe should not
    // crash the UI - it just means connection-changed feedback is
    // slower (we'll see it on the next explicit get_connection_state).
    try {
      await listen<ConnectionStateInfo>("connection-changed", (e) => {
        connection.value = e.payload;
      });
      await listen<{ action: string; ok: boolean; error?: string }>(
        "sync-finished",
        (e) => {
          const action = e.payload.action;
          const key = `sync-${action}`;
          if (e.payload.ok) {
            ops.value[key] = { state: "success" };
          } else {
            ops.value[key] = {
              state: "error",
              errorCode: "SYNC_FAILED",
              errorMessage: e.payload.error ?? "sync failed",
            };
          }
        },
      );
    } catch {
      // ignore - Tauri may not be available in test envs
    }
  }

  async function refreshPorts() {
    const r = await C.listUsbPorts();
    if (r.ok) usbPorts.value = r.value;
  }

  async function connectUsb(port: string) {
    ops.value.connect = { state: "running" };
    const r = await C.connectUsb(port);
    if (r.ok) {
      ops.value.connect = { state: "success" };
      const cs = await C.getConnectionState();
      if (cs.ok) connection.value = cs.value;
    } else {
      ops.value.connect = {
        state: "error",
        errorCode: r.error.code,
        errorMessage: r.error.message,
      };
    }
    return r;
  }

  async function disconnect() {
    ops.value.disconnect = { state: "running" };
    const r = await C.disconnectDevice();
    if (r.ok) {
      ops.value.disconnect = { state: "success" };
      const cs = await C.getConnectionState();
      if (cs.ok) connection.value = cs.value;
    } else {
      ops.value.disconnect = {
        state: "error",
        errorCode: r.error.code,
        errorMessage: r.error.message,
      };
    }
    return r;
  }

  async function discoverBle(): Promise<boolean> {
    ops.value.bleScan = { state: "running" };
    const r = await C.discoverBle();
    if (r.ok) {
      ops.value.bleScan = { state: r.value ? "success" : "idle" };
    } else {
      ops.value.bleScan = {
        state: "error",
        errorCode: r.error.code,
        errorMessage: r.error.message,
      };
    }
    return r.ok && r.value;
  }

  async function connectBle() {
    ops.value.connect = { state: "running" };
    const r = await C.connectBle();
    if (r.ok) {
      ops.value.connect = { state: "success" };
      const cs = await C.getConnectionState();
      if (cs.ok) connection.value = cs.value;
    } else {
      ops.value.connect = {
        state: "error",
        errorCode: r.error.code,
        errorMessage: r.error.message,
      };
    }
    return r;
  }

  async function run<T>(key: string, fn: () => Promise<C.Result<T>>): Promise<C.Result<T>> {
    ops.value[key] = { state: "running" };
    const r = await fn();
    if (r.ok) {
      ops.value[key] = { state: "success" };
    } else {
      ops.value[key] = {
        state: "error",
        errorCode: r.error.code,
        errorMessage: r.error.message,
      };
    }
    return r;
  }

  function setDeviceStatusFromCommand(res: DeviceCommandResult) {
    lastResult.value = res;
    lastReplyAt.value = Date.now();
    if (res.status) deviceStatus.value = res.status;
  }

  const isConnected = computed(() => connection.value.connected);
  const connectionKind = computed(() => connection.value.kind);

  return {
    connection,
    usbPorts,
    deviceStatus,
    lastReplyAt,
    lastResult,
    ops,
    isConnected,
    connectionKind,
    bootstrap,
    refreshPorts,
    connectUsb,
    disconnect,
    discoverBle,
    connectBle,
    run,
    setDeviceStatusFromCommand,
  };
});
