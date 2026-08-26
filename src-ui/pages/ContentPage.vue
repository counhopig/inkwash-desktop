<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import Frame from "../components/Frame.vue";
import Field from "../components/Field.vue";
import Button from "../components/Button.vue";
import Notice from "../components/Notice.vue";
import EmptyState from "../components/EmptyState.vue";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import { useServerStore } from "../stores/server";
import { validateAlarm, validateTodo, isAlarmFormValid, isTodoFormValid } from "../lib/validation";
import type { Alarm, AlarmInput, Channel, Importance, InboxItem, Repeat, Todo, TodoDue, TodoInput } from "../lib/types";

const server = useServerStore();

const tab = ref<"connection" | "content">("connection");

const newDeviceName = ref("");
const registerError = ref<string | null>(null);
const registeredToken = ref<string | null>(null);

const showServerToken = ref(false);
const newChannelName = ref("");
const channelError = ref<string | null>(null);
const createdChannelToken = ref<{ channel: Channel; token: string; url: string } | null>(null);

const confirmClearAlarms = ref(false);
const confirmClearTodos = ref(false);
const confirmClearInbox = ref(false);
const newAlarm = ref<AlarmInput>({
  hour: 7,
  minute: 0,
  label: "",
  repeat: "Daily",
  enabled: true,
});
const editingAlarmId = ref<number | null>(null);
const newTodo = ref<TodoInput>({ text: "", done: false, importance: "medium", dueDate: null, repeat: null });
const newTodoDueText = ref("");
const newTodoRepeatKind = ref<"" | "Daily" | "Weekly" | "Monthly">("");
const newTodoRepeatDays = ref("");
const editingTodoId = ref<number | null>(null);

onMounted(async () => {
  if (server.baseUrl && server.adminToken) {
    await server.refreshDevices();
    if (server.selectedDeviceId != null) await server.refreshContent();
  }
});

async function connect() {
  await server.refreshDevices();
  if (server.connected && server.selectedDeviceId != null) {
    await server.refreshContent();
  }
}

async function register() {
  registerError.value = null;
  registeredToken.value = null;
  const name = newDeviceName.value.trim();
  if (!name) {
    registerError.value = "Name must not be empty";
    return;
  }
  const r = await server.registerDevice(name);
  if (r.ok) {
    newDeviceName.value = "";
    registeredToken.value = r.value.token ?? null;
    server.selectDevice(r.value.id);
    await server.refreshContent();
  } else {
    registerError.value = `${r.error.code}: ${r.error.message}`;
  }
}

function copyRegisteredToken() {
  if (registeredToken.value) navigator.clipboard.writeText(registeredToken.value).catch(() => {});
}

async function unregister(id: string) {
  await server.deleteDevice(id);
}

async function pick(id: string) {
  registeredToken.value = null;
  server.selectDevice(id);
  await server.refreshContent();
}

const alarmErrors = computed(() => validateAlarm(newAlarm.value));
const todoErrors = computed(() => validateTodo(newTodo.value));

async function addAlarm() {
  if (!isAlarmFormValid(newAlarm.value)) return;
  await server.createAlarm({ ...newAlarm.value });
  newAlarm.value = { hour: 7, minute: 0, label: "", repeat: "Daily", enabled: true };
}

function startEditAlarm(a: Alarm) {
  editingAlarmId.value = a.id;
  newAlarm.value = {
    hour: a.hour,
    minute: a.minute,
    label: a.label,
    repeat: a.repeat,
    enabled: a.enabled,
  };
  applyRepeatToAlarmForm(a);
}

async function saveAlarm() {
  if (editingAlarmId.value == null) return;
  if (!isAlarmFormValid(newAlarm.value)) return;
  await server.updateAlarm(editingAlarmId.value, { ...newAlarm.value });
  editingAlarmId.value = null;
  newAlarm.value = { hour: 7, minute: 0, label: "", repeat: "Daily", enabled: true };
}

function cancelAlarmEdit() {
  editingAlarmId.value = null;
  newAlarm.value = { hour: 7, minute: 0, label: "", repeat: "Daily", enabled: true };
}

async function toggleAlarmEnabled(a: Alarm) {
  await server.updateAlarm(a.id, { ...a, enabled: !a.enabled });
}

async function removeAlarm(id: number) {
  await server.deleteAlarm(id);
}

async function doClearAlarms() {
  await server.clearAlarms();
  confirmClearAlarms.value = false;
}

function parseDueDate(raw: string): TodoDue | null {
  const m = raw.trim().match(/^(\d{4})-(\d{1,2})-(\d{1,2})$/);
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  if (month < 1 || month > 12 || day < 1 || day > 31) return null;
  return { year, month, day };
}

function dueText(t: Todo | null): string {
  if (!t?.due_date) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${t.due_date.year}-${pad(t.due_date.month)}-${pad(t.due_date.day)}`;
}

function buildRepeat(kind: string, daysRaw: string): Repeat | null {
  if (kind === "Weekly" || kind === "Monthly") {
    const days = daysRaw
      .split(",")
      .map((s) => Number(s.trim()))
      .filter((n) => Number.isFinite(n) && n >= 0 && n <= 31);
    if (days.length === 0) return null;
    return kind === "Weekly" ? { Weekly: { days } } : { Monthly: { days } };
  }
  if (kind === "Daily") return "Daily";
  return null;
}

function repeatKindOf(r: Repeat | null): "" | "Daily" | "Weekly" | "Monthly" {
  if (!r) return "";
  if (r === "Daily") return "Daily";
  if ("Weekly" in r) return "Weekly";
  if ("Monthly" in r) return "Monthly";
  return "";
}

function repeatDaysOf(r: Repeat | null): string {
  if (!r || r === "Daily") return "";
  if ("Weekly" in r) return r.Weekly.days.join(",");
  if ("Monthly" in r) return r.Monthly.days.join(",");
  return "";
}

function repeatLabel(r: Repeat | null): string {
  if (!r) return "once";
  if (r === "Daily") return "daily";
  if ("Weekly" in r) return `weekly [${r.Weekly.days.map((d) => ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"][d]).join(",")}]`;
  if ("Monthly" in r) return `monthly [${r.Monthly.days.join(",")}]`;
  if ("Once" in r) return `once ${r.Once.year}-${String(r.Once.month).padStart(2, "0")}-${String(r.Once.day).padStart(2, "0")}`;
  return "";
}

async function addTodo() {
  if (!isTodoFormValid(newTodo.value)) return;
  const due = newTodoDueText.value.trim();
  const dueDate = due ? parseDueDate(due) : null;
  if (due && !dueDate) {
    return;
  }
  const repeat = buildRepeat(newTodoRepeatKind.value, newTodoRepeatDays.value);
  await server.createTodo({
    text: newTodo.value.text.trim(),
    done: false,
    importance: newTodo.value.importance,
    dueDate,
    repeat,
  });
  resetTodoForm();
}

function resetTodoForm() {
  newTodo.value = { text: "", done: false, importance: "medium", dueDate: null, repeat: null };
  newTodoDueText.value = "";
  newTodoRepeatKind.value = "";
  newTodoRepeatDays.value = "";
}

function startEditTodo(t: Todo) {
  editingTodoId.value = t.id;
  newTodo.value = { text: t.text, done: t.done, importance: t.importance, dueDate: t.due_date, repeat: t.repeat };
  newTodoDueText.value = dueText(t);
  newTodoRepeatKind.value = repeatKindOf(t.repeat);
  newTodoRepeatDays.value = repeatDaysOf(t.repeat);
}

async function saveTodo() {
  if (editingTodoId.value == null) return;
  if (!isTodoFormValid(newTodo.value)) return;
  const due = newTodoDueText.value.trim();
  const dueDate = due ? parseDueDate(due) : null;
  if (due && !dueDate) {
    return;
  }
  const repeat = buildRepeat(newTodoRepeatKind.value, newTodoRepeatDays.value);
  await server.updateTodo(editingTodoId.value, {
    text: newTodo.value.text.trim(),
    done: newTodo.value.done,
    importance: newTodo.value.importance,
    dueDate,
    repeat,
  });
  editingTodoId.value = null;
  resetTodoForm();
}

function cancelTodoEdit() {
  editingTodoId.value = null;
  resetTodoForm();
}

async function toggleTodoDone(t: Todo) {
  await server.updateTodo(t.id, {
    text: t.text,
    done: !t.done,
    importance: t.importance,
    dueDate: t.due_date,
    repeat: t.repeat,
  });
}

async function removeTodo(id: number) {
  await server.deleteTodo(id);
}

function importanceLabel(i: Importance): string {
  return i === "high" ? "High" : i === "medium" ? "Med" : "Low";
}

function importanceClass(i: Importance): string {
  return i === "high" ? "warn" : i === "medium" ? "pending" : "ok";
}

async function doClearTodos() {
  await server.clearTodos();
  confirmClearTodos.value = false;
}

async function createChannel() {
  channelError.value = null;
  const name = newChannelName.value.trim();
  if (!name) {
    channelError.value = "Channel name must not be empty";
    return;
  }
  const r = await server.createWebhookChannel(name);
  if (r.ok) {
    newChannelName.value = "";
    createdChannelToken.value = r.value.token
      ? { channel: r.value.channel, token: r.value.token, url: r.value.delivery_url ?? "" }
      : null;
  } else {
    channelError.value = `${r.error.code}: ${r.error.message}`;
  }
}

async function removeChannel(c: Channel) {
  await server.deleteChannel(c.id);
}

async function rotateChannel(c: Channel) {
  const r = await server.rotateChannelToken(c.id);
  if (r.ok && r.value) {
    createdChannelToken.value = { channel: c, token: r.value, url: `/api/channels/${c.id}/messages` };
  }
}

function copyChannelToken() {
  if (createdChannelToken.value) navigator.clipboard.writeText(createdChannelToken.value.token).catch(() => {});
}

async function removeInboxItem(seq: number) {
  await server.deleteInboxItem(seq);
}

async function doClearInbox() {
  await server.clearInbox();
  confirmClearInbox.value = false;
}

function inboxTime(m: InboxItem): string {
  if (!m.when) return "";
  const d = new Date(m.when * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function formatAlarm(a: Alarm): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(a.hour)}:${pad(a.minute)}`;
}

type RepeatKind = "Daily" | "Weekly" | "Monthly" | "Once";
const newAlarmRepeatKind = ref<RepeatKind>("Daily");
const newAlarmRepeatDays = ref("");
const newAlarmOnceDate = ref<string>("");

function syncRepeatKind() {
  const kind = newAlarmRepeatKind.value;
  if (kind === "Daily") {
    newAlarm.value.repeat = "Daily";
  } else if (kind === "Weekly" || kind === "Monthly") {
    const days = newAlarmRepeatDays.value
      .split(",")
      .map((s) => Number(s.trim()))
      .filter((n) => Number.isFinite(n) && n >= 0 && n <= 31);
    if (days.length > 0) {
      newAlarm.value.repeat =
        kind === "Weekly" ? { Weekly: { days } } : { Monthly: { days } };
    }
  } else {
    const [y, m, d] = newAlarmOnceDate.value.split("-").map(Number);
    if (y && m && d) newAlarm.value.repeat = { Once: { year: y, month: m, day: d } };
  }
}
function onRepeatKindChange() {
  syncRepeatKind();
}
function onRepeatDaysChange() {
  syncRepeatKind();
}
function onOnceDateChange() {
  syncRepeatKind();
}

function applyRepeatToAlarmForm(a: Alarm) {
  newAlarmRepeatKind.value = "Daily";
  newAlarmRepeatDays.value = "";
  newAlarmOnceDate.value = "";
  const r = a.repeat;
  if (r === "Daily") {
    newAlarmRepeatKind.value = "Daily";
  } else if ("Weekly" in r) {
    newAlarmRepeatKind.value = "Weekly";
    newAlarmRepeatDays.value = r.Weekly.days.join(",");
  } else if ("Monthly" in r) {
    newAlarmRepeatKind.value = "Monthly";
    newAlarmRepeatDays.value = r.Monthly.days.join(",");
  } else {
    newAlarmRepeatKind.value = "Once";
    const pad = (n: number) => String(n).padStart(2, "0");
    newAlarmOnceDate.value = `${r.Once.year}-${pad(r.Once.month)}-${pad(r.Once.day)}`;
  }
}
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div>
        <h1 class="page-title">Content</h1>
        <p class="page-subtitle">
          Server is the source of truth. The device pulls Alarm/Todo content on each sync.
        </p>
      </div>
    </header>

    <div class="tabs" role="tablist">
      <button
        v-for="t in ([
          { key: 'connection', label: 'Connection' },
          { key: 'content', label: 'Content' },
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
      <div class="col-8">
        <Frame title="Server connection" subtitle="Admin token bearer auth">
          <div class="field-row">
            <Field label="Server URL" style="flex: 1.4;">
              <input
                :value="server.baseUrl"
                @input="(e) => server.setBaseUrl((e.target as HTMLInputElement).value)"
                placeholder="http://192.168.1.10:8080"
              />
            </Field>
            <Field label="Admin token" hint="Stored in localStorage on this machine only.">
              <div class="row" style="gap: 0;">
                <input
                  :value="server.adminToken"
                  @input="(e) => server.setAdminToken((e.target as HTMLInputElement).value)"
                  :type="showServerToken ? 'text' : 'password'"
                  placeholder="paste admin token"
                  style="flex: 1;"
                />
                <Button size="small" variant="ghost" @click="showServerToken = !showServerToken">
                  {{ showServerToken ? "Hide" : "Show" }}
                </Button>
              </div>
            </Field>
          </div>
          <div class="row between" style="margin-top: var(--s-3);">
            <StatusMark
              :status="server.connected ? 'ok' : 'idle'"
              :label="server.connected ? 'linked to server' : 'not linked'"
            />
            <Button variant="primary" @click="connect">Connect &amp; list devices</Button>
          </div>
          <Notice v-if="server.lastError" variant="error" :title="server.lastError.code">
            {{ server.lastError.message }}
          </Notice>
        </Frame>
      </div>

      <div class="col-4">
        <Frame title="Register device" subtitle="Server returns a one-time device token">
          <Field label="Device name">
            <input v-model="newDeviceName" type="text" placeholder="e.g. Living room" maxlength="64" />
          </Field>
          <div class="row end">
            <Button variant="primary" :disabled="!server.connected" @click="register">Register</Button>
          </div>
          <Notice v-if="registerError" variant="error">{{ registerError }}</Notice>
          <Notice v-if="registeredToken" variant="info" title="Device token (shown once)">
            <div class="row" style="gap: var(--s-2); align-items: flex-start;">
              <div style="flex: 1; min-width: 0; font-family: var(--font-mono); font-size: var(--t-12); word-break: break-all;">
                {{ registeredToken }}
              </div>
              <Button size="small" variant="ghost" @click="copyRegisteredToken">Copy</Button>
            </div>
            <div class="hint" style="margin-top: var(--s-2);">
              Paste into Device → Sync server → Device token. It will not be shown again.
            </div>
          </Notice>
        </Frame>
      </div>

      <div class="col-12">
        <Frame title="Devices">
          <EmptyState v-if="server.devices.length === 0" glyph="·" title="No devices yet">
            Register a device on the right, then pick it below to manage its content.
          </EmptyState>
          <div v-else class="row-list">
            <div
              v-for="d in server.devices"
              :key="d.id"
              :class="['row-item', { selected: d.id === server.selectedDeviceId }]"
              @click="pick(d.id)"
            >
              <div class="body">
                <div class="title">{{ d.name }}</div>
              </div>
              <div class="actions">
                <Button size="small" variant="ghost" @click.stop="unregister(d.id)">Delete</Button>
              </div>
            </div>
          </div>
        </Frame>
      </div>
    </section>

    <section v-else class="page-grid">
      <div class="col-6">
        <Frame
          :title="server.selectedDevice ? `Alarms · ${server.selectedDevice.name}` : 'Alarms'"
          :subtitle="server.selectedDevice ? 'device content' : 'select a device'"
        >
          <div class="row" style="align-items: flex-end; flex-wrap: wrap; gap: var(--s-3);">
            <Field label="Hour">
              <input v-model.number="newAlarm.hour" type="number" min="0" max="23" />
            </Field>
            <Field label="Minute">
              <input v-model.number="newAlarm.minute" type="number" min="0" max="59" />
            </Field>
            <Field label="Label" style="min-width: 160px; flex: 1;">
              <input v-model="newAlarm.label" type="text" maxlength="32" placeholder="(optional)" />
            </Field>
            <Field label="Repeat">
              <select v-model="newAlarmRepeatKind" @change="onRepeatKindChange">
                <option value="Daily">Daily</option>
                <option value="Weekly">Weekly</option>
                <option value="Monthly">Monthly</option>
                <option value="Once">Once</option>
              </select>
            </Field>
            <Field v-if="newAlarmRepeatKind === 'Weekly' || newAlarmRepeatKind === 'Monthly'" label="Days">
              <input
                v-model="newAlarmRepeatDays"
                type="text"
                :placeholder="newAlarmRepeatKind === 'Weekly' ? '0,2,4 (0=Sun)' : '1,15'"
                style="width: 120px;"
                @change="onRepeatDaysChange"
              />
            </Field>
            <Field v-if="newAlarmRepeatKind === 'Once'" label="Date">
              <input v-model="newAlarmOnceDate" type="date" @change="onOnceDateChange" />
            </Field>
            <Field label="Enabled">
              <select v-model="newAlarm.enabled">
                <option :value="true">On</option>
                <option :value="false">Off</option>
              </select>
            </Field>
            <div class="row end" style="flex: 0 0 auto;">
              <Button
                v-if="editingAlarmId == null"
                variant="primary"
                :disabled="!server.connected || !server.selectedDevice || !isAlarmFormValid(newAlarm)"
                @click="addAlarm"
              >
                Add alarm
              </Button>
              <template v-else>
                <Button variant="ghost" @click="cancelAlarmEdit">Cancel</Button>
                <Button variant="primary" :disabled="!isAlarmFormValid(newAlarm)" @click="saveAlarm">Save</Button>
              </template>
            </div>
          </div>

          <p v-if="Object.keys(alarmErrors).length" class="hint" style="color: var(--ink);">
            Fix: {{ Object.values(alarmErrors).join("; ") }}
          </p>

          <EmptyState v-if="server.alarms.length === 0" glyph="·" title="No alarms on the server">
            Add one above. Devices will pick it up on the next sync.
          </EmptyState>
          <div v-else class="row-list">
            <div v-for="a in server.alarms" :key="a.id" class="row-item">
              <div class="body">
                <div class="title">{{ formatAlarm(a) }} · {{ a.label || "(no label)" }}</div>
                <div class="meta">
                  #{{ a.id }} · {{ repeatLabel(a.repeat) }} ·
                  <span :class="['mark', a.enabled ? 'ok' : 'idle']">{{ a.enabled ? "enabled" : "disabled" }}</span>
                </div>
              </div>
              <div class="actions">
                <Button size="small" variant="ghost" @click="toggleAlarmEnabled(a)">
                  {{ a.enabled ? "Disable" : "Enable" }}
                </Button>
                <Button size="small" variant="ghost" @click="startEditAlarm(a)">Edit</Button>
                <Button size="small" variant="danger" @click="removeAlarm(a.id)">Delete</Button>
              </div>
            </div>
          </div>
          <div class="row end" style="margin-top: var(--s-3);">
            <Button
              variant="danger"
              :disabled="!server.connected || !server.selectedDevice || server.alarms.length === 0"
              @click="confirmClearAlarms = true"
            >
              Clear all alarms
            </Button>
          </div>
        </Frame>
      </div>

      <div class="col-6">
        <Frame
          :title="server.selectedDevice ? `Todos · ${server.selectedDevice.name}` : 'Todos'"
          :subtitle="`${server.todoDoneCount} done · ${server.todoCount - server.todoDoneCount} pending`"
        >
          <div class="row wrap" style="align-items: flex-end; gap: var(--s-3);">
            <Field label="New todo" style="flex: 1; min-width: 240px;">
              <input v-model="newTodo.text" type="text" maxlength="200" placeholder="What needs doing?" />
            </Field>
            <Field label="Importance">
              <select v-model="newTodo.importance">
                <option value="low">Low</option>
                <option value="medium">Medium</option>
                <option value="high">High</option>
              </select>
            </Field>
            <Field label="Due date">
              <input v-model="newTodoDueText" type="date" style="width: 150px;" />
            </Field>
            <Field label="Repeat">
              <select v-model="newTodoRepeatKind">
                <option value="">Once</option>
                <option value="Daily">Daily</option>
                <option value="Weekly">Weekly</option>
                <option value="Monthly">Monthly</option>
              </select>
            </Field>
            <Field v-if="newTodoRepeatKind === 'Weekly' || newTodoRepeatKind === 'Monthly'" label="Days">
              <input
                v-model="newTodoRepeatDays"
                type="text"
                :placeholder="newTodoRepeatKind === 'Weekly' ? '0,2,4 (0=Sun)' : '1,15'"
                style="width: 100px;"
              />
            </Field>
            <div class="row end">
              <Button
                v-if="editingTodoId == null"
                variant="primary"
                :disabled="!server.connected || !server.selectedDevice || !isTodoFormValid(newTodo)"
                @click="addTodo"
              >
                Add todo
              </Button>
              <template v-else>
                <Button variant="ghost" @click="cancelTodoEdit">Cancel</Button>
                <Button variant="primary" :disabled="!isTodoFormValid(newTodo)" @click="saveTodo">Save</Button>
              </template>
            </div>
          </div>
          <p v-if="Object.keys(todoErrors).length" class="hint" style="color: var(--ink);">
            Fix: {{ Object.values(todoErrors).join("; ") }}
          </p>
          <EmptyState v-if="server.todos.length === 0" glyph="·" title="No todos">
            Add the first one above.
          </EmptyState>
          <div v-else class="row-list">
            <div v-for="t in server.todos" :key="t.id" class="row-item">
              <div class="body">
                <div class="title">
                  <label class="checkbox" @click.stop>
                    <input type="checkbox" :checked="t.done" @change="toggleTodoDone(t)" />
                    <span :style="{ textDecoration: t.done ? 'line-through' : 'none', color: t.done ? 'var(--ink-muted)' : 'var(--ink)' }">
                      {{ t.text }}
                    </span>
                  </label>
                </div>
                <div class="meta">
                  <span :class="['mark', importanceClass(t.importance)]">{{ importanceLabel(t.importance) }}</span>
                  <span v-if="t.due_date">due {{ dueText(t) }}</span>
                  <span v-if="t.repeat">· {{ repeatLabel(t.repeat) }}</span>
                  <span>· <span :class="['mark', t.done ? 'ok' : 'pending']">{{ t.done ? "done" : "pending" }}</span></span>
                </div>
              </div>
              <div class="actions">
                <Button size="small" variant="ghost" @click="startEditTodo(t)">Edit</Button>
                <Button size="small" variant="danger" @click="removeTodo(t.id)">Delete</Button>
              </div>
            </div>
          </div>
          <div class="row end" style="margin-top: var(--s-3);">
            <Button
              variant="danger"
              :disabled="!server.connected || !server.selectedDevice || server.todos.length === 0"
              @click="confirmClearTodos = true"
            >
              Clear all todos
            </Button>
          </div>
        </Frame>
      </div>

      <div class="col-6">
        <Frame
          :title="server.selectedDevice ? `Channels · ${server.selectedDevice.name}` : 'Channels'"
          subtitle="Sources that push notifications to this device"
        >
          <div class="row" style="align-items: flex-end; gap: var(--s-3); flex-wrap: wrap;">
            <Field label="New webhook channel" style="flex: 1; min-width: 200px;">
              <input v-model="newChannelName" type="text" maxlength="80" placeholder="e.g. CI" @keyup.enter="createChannel" />
            </Field>
            <div class="row end">
              <Button
                variant="primary"
                :disabled="!server.connected || !server.selectedDevice || !newChannelName.trim()"
                @click="createChannel"
              >
                Create
              </Button>
            </div>
          </div>
          <p v-if="channelError" class="hint" style="color: var(--ink);">Error: {{ channelError }}</p>

          <div v-if="createdChannelToken" class="box">
            <div class="title">Webhook token (shown once) — copy it now</div>
            <div class="token-line">{{ createdChannelToken.token }}</div>
            <div class="meta">Delivery URL: {{ createdChannelToken.url }}</div>
            <div class="row" style="gap: var(--s-2); margin-top: var(--s-2);">
              <Button size="small" variant="primary" @click="copyChannelToken">Copy token</Button>
              <Button size="small" variant="ghost" @click="createdChannelToken = null">Dismiss</Button>
            </div>
          </div>

          <EmptyState v-if="server.channels.length === 0" glyph="·" title="No channels yet">
            Create a webhook channel above, then POST messages to its delivery URL.
          </EmptyState>
          <div v-else class="row-list" style="margin-top: var(--s-3);">
            <div v-for="c in server.channels" :key="c.id" class="row-item">
              <div class="body">
                <div class="title">{{ c.name }}</div>
                <div class="meta">
                  {{ c.kind }} · token {{ c.token_prefix }}… ·
                  <span :class="['mark', c.enabled ? 'ok' : 'idle']">{{ c.enabled ? "enabled" : "disabled" }}</span>
                </div>
              </div>
              <div class="actions">
                <Button size="small" variant="ghost" @click="rotateChannel(c)">Rotate</Button>
                <Button size="small" variant="danger" @click="removeChannel(c)">Delete</Button>
              </div>
            </div>
          </div>
        </Frame>
      </div>

      <div class="col-6">
        <Frame
          :title="server.selectedDevice ? `Inbox · ${server.selectedDevice.name}` : 'Inbox'"
          :subtitle="`${server.inbox.filter((m) => !m.read).length} unread · ${server.inbox.length} total`"
        >
          <EmptyState v-if="server.inbox.length === 0" glyph="·" title="No inbox messages">
            Messages delivered to this device's webhook channel appear here.
          </EmptyState>
          <div v-else class="row-list">
            <div v-for="m in server.inbox" :key="m.id" class="row-item">
              <div class="body">
                <div class="title">
                  <span :class="['mark', m.read ? 'idle' : 'ok']">{{ m.read ? "read" : "new" }}</span>
                  <span v-if="m.priority === 'high'" class="mark warn">!! urgent</span>
                  {{ m.title }}
                </div>
                <div class="meta">{{ m.kind }}{{ inboxTime(m) ? " · " + inboxTime(m) : "" }}</div>
                <div v-if="m.body" class="meta">{{ m.body }}</div>
              </div>
              <div class="actions">
                <Button size="small" variant="danger" @click="removeInboxItem(m.id)">Delete</Button>
              </div>
            </div>
          </div>
          <div class="row end" style="margin-top: var(--s-3);">
            <Button
              variant="danger"
              :disabled="!server.connected || !server.selectedDevice || server.inbox.length === 0"
              @click="confirmClearInbox = true"
            >
              Clear read messages
            </Button>
          </div>
        </Frame>
      </div>
    </section>

    <ConfirmDialog
      :open="confirmClearAlarms"
      title="Clear all alarms on the server?"
      description="This permanently deletes every alarm currently stored for this device on the server. The device will pick up the empty state on its next sync."
      confirm-label="Clear all"
      destructive
      @cancel="confirmClearAlarms = false"
      @confirm="doClearAlarms"
    />

    <ConfirmDialog
      :open="confirmClearTodos"
      title="Clear all todos on the server?"
      description="This permanently deletes every todo currently stored for this device on the server."
      confirm-label="Clear all"
      destructive
      @cancel="confirmClearTodos = false"
      @confirm="doClearTodos"
    />

    <ConfirmDialog
      :open="confirmClearInbox"
      title="Clear read inbox messages?"
      description="This deletes every read message currently stored for this device on the server. Unread messages are kept."
      confirm-label="Clear read"
      destructive
      @cancel="confirmClearInbox = false"
      @confirm="doClearInbox"
    />
  </div>
</template>
