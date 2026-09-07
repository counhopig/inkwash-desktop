<script setup lang="ts">
import { computed, ref, onMounted, watch } from "vue";
import Frame from "../components/Frame.vue";
import Field from "../components/Field.vue";
import Button from "../components/Button.vue";
import Notice from "../components/Notice.vue";
import StatusMark from "../components/StatusMark.vue";
import EmptyState from "../components/EmptyState.vue";
import { useDeviceStore } from "../stores/device";
import { useServerStore } from "../stores/server";
import {
  listCommonTimezones,
  systemTimezoneOffsetMinutes,
  formatUtcOffset,
  tzLabel,
  isInsecureHttpUrl,
} from "../lib/format";
import { validatePassword, validateSsid, validateUrl, validateToken, validateTimezone } from "../lib/validation";
import {
  getDeviceStatus,
  setWifi,
  setServer,
  setTimezone,
  syncNow,
  clearDeviceAlarms,
  scanWifiNetworks,
} from "../lib/commands";
import type { WifiNetwork } from "../lib/types";

const device = useDeviceStore();
const server = useServerStore();
const refreshing = ref(false);

const tab = ref<"connection" | "settings">("connection");

const selectedPort = ref("");
const ssid = ref("");
const password = ref("");
const showPassword = ref(false);

// PC-side 2.4 GHz scan state. Independent of the device connection - the
// scan runs on this computer and only fills the SSID field below.
const scanning = ref(false);
const scannedNetworks = ref<WifiNetwork[]>([]);
const scanError = ref("");

async function scanNetworks() {
  scanning.value = true;
  scanError.value = "";
  try {
    const r = await scanWifiNetworks();
    if (r.ok) {
      scannedNetworks.value = r.value;
    } else {
      scannedNetworks.value = [];
      scanError.value = r.error.detail ? `${r.error.message}: ${r.error.detail}` : r.error.message;
    }
  } finally {
    scanning.value = false;
  }
}

function pickNetwork(net: WifiNetwork) {
  ssid.value = net.ssid;
  scannedNetworks.value = [];
}
// Seed from the admin connection already configured on the Content page -
// it's the same server/port, already proven reachable there, so retyping
// it by hand can't drop the port. This field is NOT the admin base URL
// though: the firmware POSTs to it verbatim with no path construction of
// its own (see sync.rs's fetch_and_apply), so it must be the full sync
// endpoint - append /api/sync, matching the route the server actually
// accepts POSTs on (routes.rs mounts device_push_sync at "/api/sync";
// "/" only serves the admin page and returns 405 on POST).
function deriveSyncUrl(adminBaseUrl: string): string {
  if (!adminBaseUrl) return "";
  return `${adminBaseUrl.replace(/\/+$/, "")}/api/sync`;
}
const serverUrl = ref(deriveSyncUrl(server.baseUrl));
const serverToken = ref("");
const tzOffset = ref<number>(systemTimezoneOffsetMinutes());

watch(
  () => device.usbPorts,
  (ports) => {
    if (!selectedPort.value && ports.length > 0) selectedPort.value = ports[0];
  },
  { immediate: true },
);

const ssidError = computed(() => (ssid.value ? validateSsid(ssid.value) : null));
const passwordError = computed(() => (password.value ? validatePassword(password.value) : null));
const urlError = computed(() => (serverUrl.value ? validateUrl(serverUrl.value) : null));
const urlWarning = computed(() =>
  isInsecureHttpUrl(serverUrl.value)
    ? "HTTP sends the device token without transport encryption. HTTPS is recommended."
    : null,
);
const tokenError = computed(() => validateToken(serverToken.value));
const tzError = computed(() => validateTimezone(tzOffset.value));

const usbConnected = computed(() => device.connection.connected && device.connection.kind === "USB");
const bleConnected = computed(() => device.connection.connected && device.connection.kind === "BLE");
const bleCommandWaiting = computed(() =>
  bleConnected.value && ["status", "wifi", "server", "timezone", "sync", "clear-alarms"].some((key) => device.ops[key]?.state === "running"),
);
const connectingUsb = computed(() => device.ops.usbConnect?.state === "running");
const connectingBle = computed(() => device.ops.bleConnect?.state === "running");
const bleScanning = computed(() => device.ops.bleScan?.state === "running");
const bleBusy = computed(() => bleScanning.value || connectingBle.value);
const usbStatus = computed(() => connectingUsb.value ? "pending" : usbConnected.value ? "ok" : "idle");
const usbStatusLabel = computed(() => (connectingUsb.value ? "Connecting" : usbConnected.value ? "Connected" : "Disconnected"));
const bleStatus = computed(() => bleBusy.value ? "pending" : bleConnected.value ? "ok" : "idle");
const bleStatusLabel = computed(() => {
  if (bleScanning.value) return "Scanning";
  if (connectingBle.value) return "Connecting";
  return bleConnected.value ? "Connected" : "Disconnected";
});

onMounted(async () => {
  await device.refreshPorts();
});

async function connectUsb() {
  if (!selectedPort.value) return;
  const connected = await device.connectUsb(selectedPort.value);
  if (!connected.ok) return;
  const r = await device.run("status", getDeviceStatus);
  if (r.ok && r.value) device.setDeviceStatusFromCommand(r.value);
}

async function disconnectUsb() {
  await device.disconnect("USB");
}

async function disconnectBle() {
  await device.disconnect("BLE");
}

async function scanBle() {
  const found = await device.discoverBle();
  if (found) {
    const connected = await device.connectBle();
    if (!connected.ok) return;
    const r = await device.run("status", getDeviceStatus);
    if (r.ok && r.value) device.setDeviceStatusFromCommand(r.value);
  }
}

async function applyWifi() {
  if (ssidError.value || passwordError.value) return;
  const r = await device.run("wifi", () => setWifi(ssid.value.trim(), password.value));
  if (r.ok && r.value) device.setDeviceStatusFromCommand(r.value);
}

async function applyServer() {
  if (urlError.value || tokenError.value) return;
  const r = await device.run("server", () => setServer(serverUrl.value.trim(), serverToken.value.trim()));
  if (r.ok && r.value) device.setDeviceStatusFromCommand(r.value);
}

async function applyTimezone() {
  if (tzError.value) return;
  const r = await device.run("timezone", () => setTimezone(tzOffset.value));
  if (r.ok && r.value) device.setDeviceStatusFromCommand(r.value);
}

async function runSync() {
  await device.run("sync", syncNow);
}

async function refreshStatus() {
  refreshing.value = true;
  try {
    const r = await device.run("status", getDeviceStatus);
    if (r.ok && r.value) device.setDeviceStatusFromCommand(r.value);
  } finally {
    refreshing.value = false;
  }
}

async function clearAlarms() {
  await device.run("clear-alarms", clearDeviceAlarms);
  await refreshStatus();
}

const tzChoices = computed(() =>
  listCommonTimezones().map((t) => ({ ...t, label: tzLabel(t.name, t.offset) })),
);
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Device</h1>
        <p class="page-subtitle">Connect over USB or BLE and push Wi-Fi, server, timezone config.</p>
      </div>
      <div class="row end">
        <Button
          variant="ghost"
          :loading="refreshing"
          :disabled="!device.isConnected"
          @click="refreshStatus"
        >
          Refresh status
        </Button>
        <Button
          variant="primary"
          :loading="device.ops.sync?.state === 'running'"
          :disabled="!device.isConnected"
          @click="runSync"
        >
          Sync device now
        </Button>
      </div>
    </header>

    <Notice v-if="device.ops.sync?.state === 'error'" variant="error" :title="device.ops.sync.errorCode ?? 'Sync failed'">
      {{ device.ops.sync.errorMessage }}
    </Notice>
    <Notice v-if="bleCommandWaiting" variant="warn" title="Waiting for device">
      The device is showing a reminder or menu; this operation will apply when the screen is ready.
    </Notice>

    <div class="tabs" role="tablist">
      <button
        v-for="t in ([
          { key: 'connection', label: 'Connection' },
          { key: 'settings', label: 'Settings' },
        ] as const)"
        :key="t.key"
        type="button"
        role="tab"
        :aria-selected="tab === t.key"
        :class="['tab', { active: tab === t.key }]"
        @click="tab = t.key"
      >
        {{ t.label }}
      </button>
    </div>

    <section v-if="tab === 'connection'" class="page-grid">
      <div class="col-6">
        <Frame title="Connection" :subtitle="device.connection.kind">
          <div class="stack">
            <div class="field-row">
              <div class="field" style="flex: 2;">
                <label>USB port</label>
                <select v-model="selectedPort" :disabled="connectingUsb || device.isConnected">
                  <option v-if="device.usbPorts.length === 0" value="">No USB serial devices detected</option>
                  <option v-for="p in device.usbPorts" :key="p" :value="p">{{ p }}</option>
                </select>
                <div class="hint">Espressif VID 0x303a ports are listed first.</div>
              </div>
              <div class="row end" style="flex: 1;">
                <StatusMark :status="usbStatus" :label="usbStatusLabel" />
                <Button v-if="usbConnected" variant="danger" :loading="device.ops.usbDisconnect?.state === 'running'" @click="disconnectUsb">Disconnect USB</Button>
                <Button v-else variant="primary" :loading="connectingUsb" :disabled="!selectedPort || device.isConnected" @click="connectUsb">Connect USB</Button>
              </div>
            </div>
            <Notice v-if="device.ops.usbConnect?.state === 'error'" variant="error" :title="device.ops.usbConnect.errorCode ?? 'USB connection failed'">
              {{ device.ops.usbConnect.errorMessage }}
            </Notice>
            <Notice v-if="device.ops.usbDisconnect?.state === 'error'" variant="error" :title="device.ops.usbDisconnect.errorCode ?? 'USB disconnect failed'">
              {{ device.ops.usbDisconnect.errorMessage }}
            </Notice>

            <div class="frame-section">
              <div class="row between">
                <div>
                  <h3 style="margin:0; font-size: var(--t-14); font-weight: 600;">Bluetooth</h3>
                  <div class="hint">The Inkwash only advertises while its BLE Pairing screen is open.</div>
                </div>
                <div class="row end">
                  <StatusMark :status="bleStatus" :label="bleStatusLabel" />
                  <Button v-if="bleConnected" variant="danger" :loading="device.ops.bleDisconnect?.state === 'running'" @click="disconnectBle">
                    Disconnect BLE
                  </Button>
                  <Button v-else :loading="bleBusy" :disabled="device.isConnected" @click="scanBle">
                    {{ device.isConnected ? "Unavailable while USB is active" : "Scan for Inkwash" }}
                  </Button>
                </div>
              </div>
              <Notice v-if="device.ops.bleScan?.state === 'error'" variant="error" :title="device.ops.bleScan.errorCode ?? 'BLE scan failed'">
                {{ device.ops.bleScan.errorMessage }}
              </Notice>
              <Notice v-if="device.ops.bleConnect?.state === 'error'" variant="error" :title="device.ops.bleConnect.errorCode ?? 'BLE connection failed'">
                {{ device.ops.bleConnect.errorMessage }}
              </Notice>
              <Notice v-if="device.ops.bleDisconnect?.state === 'error'" variant="error" :title="device.ops.bleDisconnect.errorCode ?? 'BLE disconnect failed'">
                {{ device.ops.bleDisconnect.errorMessage }}
              </Notice>
            </div>
          </div>
        </Frame>
      </div>

      <div class="col-6">
        <Frame title="Device status">
          <div v-if="!device.isConnected">
            <EmptyState glyph="○" title="Not connected">
              Connect a device over USB or BLE to see live status.
            </EmptyState>
          </div>
          <div v-else class="kv">
            <div class="k">Transport</div>
            <div class="v">{{ device.connection.kind }} · {{ device.connection.port }}</div>
            <div class="k">Wi-Fi configured</div>
            <div class="v"><StatusMark :status="device.deviceStatus?.wifiConfigured ? 'ok' : 'idle'" :label="device.deviceStatus?.wifiConfigured ? 'yes' : 'no'" /></div>
            <template v-if="device.deviceStatus?.wifiSsid">
              <div class="k">Wi-Fi SSID</div>
              <div class="v">{{ device.deviceStatus.wifiSsid }}</div>
              <div class="k">Wi-Fi password</div>
              <div class="v"><StatusMark :status="device.deviceStatus.wifiHasPassword ? 'ok' : 'idle'" :label="device.deviceStatus.wifiHasPassword ? 'set' : 'none'" /></div>
            </template>
            <div class="k">Wi-Fi connected</div>
            <div class="v">
              <StatusMark :status="device.deviceStatus?.wifiConnected ? 'ok' : 'idle'" :label="device.deviceStatus?.wifiConnected ? 'yes' : 'no'" />
              <span v-if="!device.deviceStatus?.wifiConnected" class="inline-hint">connects on demand</span>
            </div>
            <div class="k">Sync server</div>
            <div class="v"><StatusMark :status="device.deviceStatus?.serverConfigured ? 'ok' : 'idle'" :label="device.deviceStatus?.serverConfigured ? 'configured' : 'not set'" /></div>
            <template v-if="device.deviceStatus?.serverUrl">
              <div class="k">Server URL</div>
              <div class="v">{{ device.deviceStatus.serverUrl }}</div>
              <div class="k">Device token</div>
              <div class="v"><StatusMark :status="device.deviceStatus.serverHasToken ? 'ok' : 'idle'" :label="device.deviceStatus.serverHasToken ? 'set' : 'none'" /></div>
            </template>
            <div class="k">Timezone</div>
            <div class="v">{{ formatUtcOffset(device.deviceStatus?.timezoneOffsetMinutes ?? 0) }}</div>
          </div>
        </Frame>
      </div>
    </section>

    <section v-else class="page-grid">
      <div class="col-6">
        <Frame title="Wi-Fi" subtitle="Pushed to the device">
          <Field label="SSID" :error="ssidError ?? undefined">
            <input v-model="ssid" type="text" placeholder="Network name" maxlength="32" />
          </Field>

          <div class="frame-section">
            <div class="row between">
              <div>
                <h3 style="margin:0; font-size: var(--t-14); font-weight: 600;">Nearby 2.4 GHz networks</h3>
                <div class="hint">Scan from this PC, then pick a network to fill the SSID above.</div>
              </div>
              <Button :loading="scanning" @click="scanNetworks">Scan</Button>
            </div>
            <p v-if="scanError" class="scan-error">{{ scanError }}</p>
            <ul v-else-if="scannedNetworks.length" class="network-list">
              <li
                v-for="net in scannedNetworks"
                :key="net.ssid + net.channel"
                type="button"
                @click="pickNetwork(net)"
              >
                <span class="net-ssid">{{ net.ssid }}</span>
                <span class="net-meta">
                  ch {{ net.channel }}
                  <template v-if="net.signal != null"> · {{ net.signal }}%</template>
                  <template v-if="net.security"> · {{ net.security }}</template>
                </span>
              </li>
            </ul>
            <p v-else-if="!scanning" class="hint" style="margin-top: var(--s-2);">
              No networks scanned yet.
            </p>
          </div>

          <Field label="Password" :error="passwordError ?? undefined" :hint="!password ? 'Empty password is allowed for open networks.' : undefined">
            <div class="row" style="gap: 0;">
              <input
                v-model="password"
                :type="showPassword ? 'text' : 'password'"
                placeholder="(empty for open networks)"
                style="flex: 1;"
                maxlength="63"
              />
              <Button size="small" variant="ghost" @click="showPassword = !showPassword">
                {{ showPassword ? "Hide" : "Show" }}
              </Button>
            </div>
          </Field>
          <div class="row end">
            <Button
              variant="primary"
              :loading="device.ops.wifi?.state === 'running'"
              :disabled="!device.isConnected || !!ssidError || !!passwordError"
              @click="applyWifi"
            >
              Save Wi-Fi
            </Button>
          </div>
          <template v-if="device.ops.wifi?.state === 'error'">
            <Notice variant="error" :title="device.ops.wifi.errorCode ?? 'Failed'">
              {{ device.ops.wifi.errorMessage }}
            </Notice>
          </template>
        </Frame>
      </div>

      <div class="col-6">
        <Frame title="Sync server" subtitle="Public URL + device token">
          <Field label="Server URL" :error="urlError ?? undefined" hint="The device POSTs to this exact URL - include the /api/sync path. HTTPS is recommended.">
            <input v-model="serverUrl" type="text" placeholder="https://example.com/api/sync" />
          </Field>
          <Notice v-if="urlWarning" variant="warn" title="HTTPS recommended">
            {{ urlWarning }}
          </Notice>
          <Field label="Device token" :error="tokenError ?? undefined" hint="Issued by the server when you register the device. Not the Admin Token.">
            <input v-model="serverToken" type="text" placeholder="paste device token here" />
          </Field>
          <div class="row end">
            <Button
              variant="primary"
              :loading="device.ops.server?.state === 'running'"
              :disabled="!device.isConnected || !!urlError || !!tokenError"
              @click="applyServer"
            >
              Save server config
            </Button>
          </div>
          <template v-if="device.ops.server?.state === 'error'">
            <Notice variant="error" :title="device.ops.server.errorCode ?? 'Failed'">
              {{ device.ops.server.errorMessage }}
            </Notice>
          </template>
        </Frame>
      </div>

      <div class="col-6">
        <Frame title="Timezone" subtitle="Sent as 15-minute offset">
          <div class="field-row">
            <Field label="Preset" hint="Pick a common zone, or override below.">
              <select
                :value="tzOffset"
                @change="(ev) => (tzOffset = Number((ev.target as HTMLSelectElement).value))"
              >
                <option v-for="t in tzChoices" :key="t.name" :value="t.offset">{{ t.label }}</option>
              </select>
            </Field>
            <Field label="UTC offset (minutes)" :error="tzError ?? undefined">
              <input v-model.number="tzOffset" type="number" step="15" min="-720" max="840" />
              <div class="hint">{{ formatUtcOffset(tzOffset) }}</div>
            </Field>
          </div>
          <div class="row end">
            <Button
              variant="primary"
              :loading="device.ops.timezone?.state === 'running'"
              :disabled="!device.isConnected || !!tzError"
              @click="applyTimezone"
            >
              Save timezone
            </Button>
          </div>
          <template v-if="device.ops.timezone?.state === 'error'">
            <Notice variant="error" :title="device.ops.timezone.errorCode ?? 'Failed'">
              {{ device.ops.timezone.errorMessage }}
            </Notice>
          </template>
        </Frame>
      </div>

      <div class="col-6">
        <Frame title="Local cleanup" subtitle="Acts on the device, not the server">
          <p class="hint">
            Clearing local alarms removes every alarm stored on the Inkwash itself. Use this before registering
            a new device or after a server reset. Server-side alarms are unaffected until the next sync.
          </p>
          <div class="row end">
            <Button
              variant="danger"
              :loading="device.ops['clear-alarms']?.state === 'running'"
              :disabled="!device.isConnected"
              @click="clearAlarms"
            >
              Clear alarms on device
            </Button>
          </div>
          <template v-if="device.ops['clear-alarms']?.state === 'error'">
            <Notice variant="error" :title="device.ops['clear-alarms'].errorCode ?? 'Failed'">
              {{ device.ops['clear-alarms'].errorMessage }}
            </Notice>
          </template>
        </Frame>
      </div>
    </section>
  </div>
</template>

<style scoped>
.scan-error {
  margin: var(--s-2) 0 0;
  font-size: var(--t-13);
  color: var(--ink-muted);
}

.network-list {
  list-style: none;
  margin: var(--s-2) 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--s-1);
  max-height: 200px;
  overflow-y: auto;
  border: var(--b-1-soft);
}

.network-list li {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--s-3);
  padding: var(--s-2) var(--s-3);
  font-size: var(--t-13);
  cursor: pointer;
  background: var(--surface-raised);
}

.network-list li:hover {
  background: var(--surface-sunken);
}

.net-ssid {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.net-meta {
  flex-shrink: 0;
  color: var(--ink-muted);
}
</style>
