# AGENTS.md — Inkwash Desktop

PC config tool for the Inkwash NOTE4 firmware (`../inkwash-firmware`), one of four repos in this workspace (`inkwash-firmware` firmware, `inkwash-server` backend, `inkwash-mcp` MCP server, this repo). English Tauri 2 app: Rust backend + Vue 3/Pinia frontend. Device wire protocol contract lives in `../inkwash-firmware/docs/control-protocol.md`; server admin API in `../inkwash-server`.

## Layout quirk

- `src/` = **Rust** (Tauri commands, USB/BLE transport, server HTTP client). `src-ui/` = **Vue** (pages/components/stores/lib/styles). Frontend is NOT in `src/`.
- Tauri commands are registered in `src/desktop.rs` (`invoke_handler`). A new command touches: the command fn, `desktop.rs`, and a typed wrapper in `src-ui/lib/commands.ts`.
- Wire types are generated: Rust wire DTOs (`src/server.rs`, `src/commands/*.rs`, `src/error.rs`) carry ts-rs `#[derive(TS)]`, and any `cargo test` run regenerates one `.ts` file per type under `src-ui/lib/generated/` (committed; CI drift gate — see Commands below). `src-ui/lib/types.ts` only re-exports them. Device-facing DTOs use `#[serde(rename_all = "camelCase")]`; the server-backed ones intentionally stay snake_case end to end (see src/server.rs module docs). u64/i64 fields are pinned to TS `number` via `#[ts(type = "number")]` - same rationale as inkwash-server's models.rs.
- `list_content` is the single alarms+todos endpoint; `list_alarms`/`list_todos` commands were deliberately removed as dead — do not re-add.

## Commands

```bash
npm run tauri dev      # dev app (vite fixed port 1420, strictPort)
npm run build          # vue-tsc --noEmit && vite build  — the only frontend check (no test framework)
cargo test             # Rust unit tests (protocol/error/logs/inflight) + live-server tests (ignored)
cargo clippy --all-targets   # lint — this repo keeps it at zero warnings
npm run tauri build    # release bundle
```

CI (`.github/workflows/ci.yml`) runs `cargo test` + `npm run build` on
every push/PR — since the build's `prebuild` hook regenerates the ts-rs
bindings, a DTO edit that forgets the committed `generated/*.ts` fails
the CI build, so the bindings now have a drift gate. The live-server
tests are skipped by default; run them against any reachable server with
`INKWASH_LIVE_URL=http://<host>:8080 cargo test -- --ignored`.

Verification order after changes: `cargo clippy --all-targets` → `cargo test` → `npm run build`. Commit messages: conventional, lowercase type (`feat(ui):`, `fix(device):`, `refactor!:`), English.

## Releases

`.github/workflows/release.yml` builds macOS/Windows/Ubuntu installers (tauri-action) and **publishes automatically** (`releaseDraft: false`) whenever a `v*` tag is pushed to the `github` remote — no manual draft handling. Linux requires `libudev-dev` (the `serialport` crate depends on it) — it is already in the apt install list; if the Linux job ever fails on `libudev-sys`, that's the missing dep.

- **Critical:** GitHub Actions runs the workflow file **at the tagged commit**, not at `main`. If you re-trigger a release by deleting + re-pushing a tag, the tag must point to a commit that already contains the latest workflow changes — otherwise the *old* workflow runs. Also delete the old release + remote tag first, because force-updating an existing tag does not reliably re-trigger the workflow:
  ```bash
  gh release delete v0.1.0 --repo counhopig/inkwash-desktop --yes
  git push github :refs/tags/v0.1.0
  git tag -f v0.1.0 <commit-with-latest-workflow>
  git push github v0.1.0
  ```
- Release check: `gh release view v0.1.0 --repo counhopig/inkwash-desktop --json isDraft,assets` (expect `isDraft: false`, one asset per platform).

## Gotchas

- **Log dir must stay outside the project tree** (`~/Library/Logs/inkwash-desktop` on macOS): `tauri dev` watches the project root, so a log file inside it triggers an infinite dev-server restart loop. Logs mirror to stderr + `device-log` Tauri event; secrets go through `redact_secret` (never log Wi-Fi passwords / admin / device tokens verbatim).
- Frontend `invoke` never throws: `lib/commands.ts` `wrap()` converts rejections to `Result<T, AppError>`; error codes come from the catalog in `src/error.rs` (e.g. `SERVER_UNAUTHORIZED`, `INVALID_INPUT`).
- Events: Rust emits `connection-changed`, `sync-finished`, `device-log`; Pinia stores (`src-ui/stores/`) are the only subscribers.
- Same binary runs headless: `inkwash-desktop --status <port> [timeout]`, `--sync`, `--ble-scan`, `--ble-list` — these never launch the window. Opening the ESP32-S3 USB serial port may reset the board, hence generous default timeouts (35/45s).
- macOS: first GUI run prompts for Bluetooth permission; deny only disables BLE, USB still works.
- Design language: "paper + ink" e-ink aesthetic. Extend styles via `src-ui/styles/tokens.css` variables, never scatter hex codes in components; status via glyphs (`○ ◉ △ ✓ ✕`), not color.
- Ignored/generated: `dist/`, `target/`, `gen/`, `node_modules/`, `logs/*.log` — never commit.
