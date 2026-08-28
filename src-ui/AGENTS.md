# AGENTS.md — src-ui (Vue 3 + Pinia frontend)

## OVERVIEW
Tauri frontend: Vue 3 `<script setup>` SFCs + Pinia stores, no router (App.vue v-if page switching). All backend access goes through typed `invoke` wrappers in `lib/commands.ts` that never throw.

## WHERE TO LOOK
| Path | Role |
|---|---|
| `App.vue` | Shell: mounts all 3 stores in `onMounted` bootstrap; switches pages by Sidebar `PageId` (`overview`/`device`/`content`/`logs`) |
| `main.ts` | createApp + pinia; imports styles in order tokens → global → layout → components |
| `pages/ContentPage.vue` (803L) | Server-side content: register/select device, alarms+todos CRUD, channels, inbox; gates forms with `validateAlarm`/`validateTodo` |
| `pages/DevicePage.vue` (485L) | USB/BLE connect, Wi-Fi scan/join, set server URL/timezone; calls `lib/commands` directly for scans/status, store for connect |
| `pages/OverviewPage.vue` (287L) | Dashboard: setup steps, next alarm, sync; reads all 3 stores, emits `navigate` |
| `pages/LogsPage.vue` (183L) | Log viewer: filter/pause/stick-to-bottom, copy, export, open folder |
| `stores/device.ts` (182L) | Connection state + per-op status; `run<T>(key, fn)` wraps every command with `OperationState` tracking |
| `stores/server.ts` (245L) | Admin URL/token, device list, selected device, alarms/todos/channels/inbox; persists via `lib/storage` |
| `stores/logs.ts` (108L) | Log buffer + filter; subscribes to `device-log` |
| `components/` (9 SFCs) | Presentational: Frame, Field, Button, StatusMark, Sidebar, TopBar, Notice, ConfirmDialog, EmptyState |
| `styles/tokens.css` (96L) | THE palette/spacing/type/border scale — see CONVENTIONS |
| `styles/global.css`, `layout.css`, `components.css` | Resets, app grid, "dumb" presentation classes |
| `lib/commands.ts` (243L) | One wrapper per Tauri command, grouped by comment (Device / Wi-Fi / Server / Logs) |
| `lib/types.ts` | Re-exports of generated types only — no handwritten wire types |
| `lib/validation.ts` (81L) | Mirrors Rust validators; fast feedback only, Rust re-rejects |
| `lib/format.ts` (66L) | Timezone table, UTC offset, `redactSecret`, time formatting |
| `lib/storage.ts` (38L) | localStorage keys `inkwash.server.*`; admin token plaintext |
| `lib/generated/` (19 ts-rs files) | Committed wire DTO bindings — never hand-edit |

## CONVENTIONS
- **Errors:** every command fn returns `Result<T, AppError>` = `{ok:true,value}|{ok:false,error}` (commands.ts `wrap()`/`normaliseError()`). Callers check `r.ok`; branch on `r.error.code` from the Rust catalog.
- **Events:** Tauri `listen()` lives only in stores — `device.ts` handles `connection-changed` + `sync-finished` (device.ts:46-67), `logs.ts` handles `device-log`. Components never subscribe.
- **Tokens:** hex only in `tokens.css`; components use `var(--*)`. Semantic colors are all the same ink on purpose (tokens.css:32-35) — differentiate by glyph/weight, never hue. Status via `StatusMark` (`idle|pending|ok|warn|fail`, default labels Offline/Pending/Connected/Attention/Failed).
- **Styling:** `components.css` classes are state-free; state/validation/events stay in SFCs. Reuse `.frame`/`.btn`/`.field` before writing new CSS.
- **Wire types:** edit the Rust `#[derive(TS)]` DTO, run `cargo test` to regenerate; snake_case on server DTOs, camelCase on device DTOs (types.ts header).
- **Validation limits** stay in sync with Rust (`SSID_MAX`/`LABEL_MAX` 32, `TODO_TEXT_MAX` 200 — validation.ts:13-15).

## ANTI-PATTERNS
- Hand-editing `lib/generated/*.ts` — ts-rs banner (generated/AppError.ts:1); CI drift gate fails on stale bindings.
- Hex codes/`rgb()` in SFCs or new semantic colors — palette is tokens only.
- `listen()` in components or bare `invoke()` — stores own event state; commands go through `lib/commands.ts`.
- Adding a router or prop-drilling — App.vue v-if switching + stores already cover navigation and shared state.
- Logging tokens/Wi-Fi passwords — `redactSecret` (format.ts:50) exists for display.

## NOTES
- No test framework; `npm run build` (vue-tsc --noEmit + vite build) is the only frontend check.
- `env.d.ts` is just the vite/client + `*.vue` shim.
- DevicePage derives the sync URL as `<adminBaseUrl>/api/sync` (DevicePage.vue:74-77) — keep in sync with server routes.
