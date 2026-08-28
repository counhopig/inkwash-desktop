# AGENTS.md — src/ (Rust backend)

## OVERVIEW
Rust core of the desktop app: CLI entry, firmware wire protocol, USB/BLE transports, admin API client, Tauri command layer. Frontend lives in `../src-ui` (see parent).

## WHERE TO LOOK
- main.rs (146L) — arg dispatch first: `--ble-scan` / `--ble-list` / `--status <port> [timeout]` / `--sync <port> [timeout]` return before the GUI (main.rs:18-40); anything else falls through to `desktop::run()` (main.rs:57). CLI reuses the GUI retry driver: `cli_usb_command` wraps `drive_request` over a bare `UsbLink` + `mpsc::Receiver`.
- desktop.rs (61L) — the entire command surface is registered in one `invoke_handler` list (desktop.rs:21+). New command touches: the fn, this list, and the typed wrapper in `../src-ui/lib/commands.ts`.
- protocol.rs (294L) — `Command`/`Reply` wire enums mirroring firmware `control.rs` (externally tagged `cmd`/`status`, snake_case variants). `encode_command` injects a top-level correlation `id`; `decode_reply` + `classify_reply` are the shared reply-matching logic for CLI and GUI retry loops — a reply with no `id` is accepted on trust (pre-correlation firmware). `next_request_id`: one id per logical request, reused across resends.
- transport/mod.rs (167L) — `Transport` trait + unified `Event` + shared `run_worker_loop` (~200ms tick: honour disconnect → drain queued commands → poll one inbound chunk). USB/BLE differ only in the `PollSource` framing half.
- transport/usb.rs (196L) — `>>IW {json}` out, `<<IW {json}` in @115200; lines without the prefix are device log noise → `Event::Log`, not discarded. `WRITE_TIMEOUT` 2s (a 200ms timeout tore down connections during Wi-Fi-association stalls). Opening the port resets the ESP32-S3.
- transport/ble.rs (310L) — GATT: write char takes bare JSON, notify char returns replies; own Tokio runtime on a worker thread (btleplug is async-only, Tauri's tokio boundary can't host it). `find_device` retries 20×.
- state.rs (318L) — `AppState` (registered in desktop.rs setup), `LinkState`, `ActiveLink` holding an `Arc` snapshot so the `link` mutex is never held across a 45s `recv_timeout` (invariant 1); `InflightRegistry`/`InflightGuard` coalesce identical concurrent requests onto one wire copy. `logs.append()` = stderr + disk + `device-log` event (invariant 2).
- error.rs (154L) — `AppError`: short `message` + stable `code`; UI never sees a raw anyhow chain (error.rs:2). Codes: DEVICE_TIMEOUT, DEVICE_DISCONNECTED, SERVER_UNREACHABLE, SERVER_UNAUTHORIZED, SERVER_ERROR, INVALID_INPUT, INTERNAL.
- server.rs (478L) — blocking `reqwest` admin client; DTOs deliberately snake_case end-to-end, u64/i64 pinned `#[ts(type = "number")]`; `Channel` never carries the plaintext token (server.rs:92), `ChannelCreated` returns it exactly once. Live-server tests at the bottom are `#[ignore]`d.
- commands/device.rs (862L, biggest) — 12 Tauri commands; two-phase send (lock → push command → drop guard → wait on snapshot); `drive_request` retry driver (resend every 2s of silence on USB, 45s deadline, `busy` → immediate resend, foreign id → `RetryNotice::StaleReply`); `send_and_wait` coalescing.
- commands/server.rs (480L) — admin CRUD, every command through `admin_call` (spawn_blocking + one `map_err` flatten); `AlarmInput`/`TodoInput` are the only camelCase DTOs here (UI-facing); `list_content` is the single alarms+todos endpoint. Commands take `base_url` + `token` per call, not from state.
- commands/scan.rs (348L) — PC-side 2.4 GHz Wi-Fi scan (the device has no scan command); per-OS backend: CoreWLAN via objc2 (needs Location Services), `netsh` parse (English + Simplified-Chinese keys, GBK decode), `nmcli -t -e no` TSV parse; filters to channels 1-14; one `WifiNetwork` shape.
- commands/logs.rs (282L) — ring buffer (4000 entries) + per-launch disk file + `device-log` event; `redact_secret` (logs.rs:216) truncates secrets before they reach stderr/disk/event; `normalise_server_url` shared with `ServerClient`.
- commands/logs_cmd.rs (64L) — read/clear/paths/open/export. `clear_logs` clears only the in-memory view, never the file; `export_log` copies to `~/Downloads`; `open_log_folder` shells to `open`/`explorer`/`xdg-open`.

## CONVENTIONS
- Every blocking call is wrapped in `tauri::async_runtime::spawn_blocking` at the command entry — the UI thread is never blocked (device.rs:15-16, server.rs:39, scan.rs:61). Sync commands (`logs_cmd`, `get_connection_state`) stay sync.
- Transport workers are plain OS threads + `std::sync::mpsc`; only BLE spins up its own Tokio runtime.
- Two-phase device sends: snapshot the link `Arc` under the mutex, release, then wait on the snapshot (never the inverse).

## ANTI-PATTERNS
- Holding the `link` mutex across `recv_timeout` — stalls every other command for up to 45s; snapshot the `Arc` first (state.rs:15-21).
- Early-returning on an error path without relying on `InflightGuard` Drop — joiners hang; Drop wakes them even when `finish` was never called (state.rs:224-227).
- camelCase-renaming server DTOs — breaks the snake_case wire contract; only UI-facing types rename (server.rs:14-19).
- A blocking `reqwest`/serialport call in the command body — always wrap in `spawn_blocking`.
