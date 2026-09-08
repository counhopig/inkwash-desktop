# Changelog

All notable changes to **inkwash-desktop** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2026-09-08

### Changed
- Version aligned with the Inkwash v0.6.0 release across the desktop,
  server, MCP, and firmware repositories.

## [0.5.0] - 2026-08-21

### Added
- **2.4 GHz Wi-Fi network scan** — the Device page's Settings tab can scan
  nearby 2.4 GHz networks from the PC and pick one to fill the SSID field.
  macOS uses CoreWLAN (Location Services permission), Windows parses
  `netsh wlan show networks mode=bssid`, Linux parses `nmcli`.

### Fixed
- **USB protocol framing** — commands were still sent with the old
  `>>IP ` / `<<IP ` prefixes after the rebrand, so the firmware (which only
  recognises `>>IW ` / `<<IW `) ignored every USB command. The wire
  prefixes now actually match `inkwash/docs/control-protocol.md`.

## [0.4.0] - 2026-08-21

### Changed
- **Rebranded to Inkwash** - package and bundle names (Inkwash Desktop),
  and the device protocol framing (`>>IW ` / `<<IW `) matching the new
  firmware.

## [0.3.0] - 2026-08-21

### Added
- **Channels & inbox management** — manage webhook channels in the
  Content page (create, copy the one-time delivery token, rotate) and
  browse / delete the device inbox messages pushed from external sources.
- **Urgent priority badge** — high-priority inbox items are marked in
  the inbox list.

### Changed
- The Content page splits channels & inbox into side-by-side panels
  instead of a single stacked list, and the completed-todo checkbox
  matches the server console's filled style.

## [0.2.0] - 2026-08-20

### Added
- Logs page shows a drafting-grid console with a device mini-screen and
  stat blocks.

### Fixed
- Log timestamps are stored as `u64` to avoid event serialization issues.
- The in-memory log buffer is resnapshotted when the Logs page opens so
  it always reflects the latest entries.
- Removed the background coordinate grid lines on the paper background,
  keeping only the faint dot grain.

### Changed
- The release workflow builds and **publishes** macOS/Windows/Ubuntu
  installers automatically on a `v*` tag (no manual draft handling).

## [0.1.0] - 2026-08-20

### Added
- Initial release: Tauri 2 + Vue 3 + TypeScript rewrite of the desktop
  configuration tool.
- Four pages: Overview (device/server state), Device (USB/BLE connect,
  Wi-Fi/server/timezone config, Sync Now), Content (register devices and
  author alarms/todos against `inkwash-server`'s admin API), and Logs
  (real-time diagnostics with secret redaction).
- USB serial and BLE transports talking the firmware's control protocol.
- Headless CLI mode: `--status <port>`, `--sync <port>`, `--ble-scan`,
  `--ble-list`.
- Platform log directories with Wi-Fi passwords / admin / device tokens
  redacted on disk.
- Unified "paper + ink" e-ink design language.
- Apache-2.0 license and open-source README.
