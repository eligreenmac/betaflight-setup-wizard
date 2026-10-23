# Architecture & Module Contracts

This document is the contract between workstreams. Anything in **§2 (shared types)**
or **§3 (command names)** may only change through a PR labelled `shared` that updates
all mirrors at once (`src-tauri/src/model.rs`, `src/types/model.ts`,
`src/api/commands.ts`, `schemas/drone-model.schema.json`, this file).

```
┌───────────────────────── UI (src/, Vue 3 + TS) ─────────────────────────┐
│ wizard/steps.json → engine.ts (visibility) → registry.ts → step .vue     │
│ wizard/state.ts (DroneModel, FcInfo, ports)     i18n/ (he RTL, en)       │
│ api/commands.ts  — typed call()/api.*  ← setBackend(mock) in tests       │
└──────────────────────────────┬───────────────────────────────────────────┘
                     Tauri invoke / events (JSON, camelCase)
┌──────────────────────────────┴───────────────────────────────────────────┐
│ lib.rs  — registers ALL commands once (shared file)                       │
│ ┌──────────┐  ┌───────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│ │ device   │◄─┤ flasher       │  │ config       │  │ system (packaging)│ │
│ │ ports    │  │ Build API     │  │ generate()   │  │ WinUSB/libwdi     │ │
│ │ serial   │  │ cache         │  │ backup/apply │  │ driver status     │ │
│ │ MSP, CLI │◄─┤ DFU, bl entry │  │ verify/undo  │──┤ app info          │ │
│ └────▲─────┘  └───────────────┘  └──────┬───────┘  └──────────────────┘  │
│      └───────── Connection trait ───────┘                                 │
│ model.rs (shared types)   error.rs (AppError {code,message})              │
└───────────────────────────────────────────────────────────────────────────┘
```

## 1. Modules and ownership

| Module | Path | Workstream | Responsibilities | Must NOT |
|---|---|---|---|---|
| device | `src-tauri/src/device` | `device` | Enumerate COM ports + VID/PID classification (`usb_ids.rs`); open/close serial (115200 8N1); MSP v1/v2 framing, request/response, timeouts; identify FC (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, UID, BUILD_INFO); CLI mode (enter `#`, exec line until prompt, exit/save); reboot to bootloader; reconnect after reboot. Owns the single active connection (`tauri::State`). Implements the `Connection` trait. | Know anything about DroneModel or firmware files. |
| flasher | `src-tauri/src/flasher` | `flasher` | Build API client (targets, releases, options, build request + poll + download); content-addressed cache with SHA-256; DFU enumeration (incl. MCU family from descriptor) and flashing (erase, write, verify, manifest/leave) for STM32 F4/F7/H7/G4 and AT32F435; orchestration: identify → backup request → `device_reboot_to_bootloader` → wait for DFU → flash → wait for VCP; progress events. | Generate CLI; install drivers (asks `system`). |
| config | `src-tauri/src/config` | `config` | `generate(DroneModel, ports) -> CliScript` (**pure**, version dialects, conflict warnings); read ports (`serial` / MSP_CF_SERIAL_CONFIG); backup (`diff all` → file); apply (`batch start` … `batch end` + `save`, collect `###ERROR###` lines); verify (read back via `get`/`serial`/`feature`/`aux`); rollback (`defaults nosave` + backup + `save`). Uses `device::Connection` only. | Open ports or talk USB directly. |
| system | `src-tauri/src/system` | `packaging` | Driver status (WinUSB on DFU IDs, VCP), install WinUSB via elevated libwdi helper, app info/paths. Also owns `tauri.conf.json` bundle section, installer, CI workflows, code signing. | Business logic. |
| ui | `src/` | `ui` | Step components, step registry, wizard state, i18n (he default, RTL), error display (`AppError.code` → i18n key), progress UI, browser mock mode. | Call `@tauri-apps/api` directly outside `src/api`. |

Shared files (any change = `shared` PR, needs review from affected owners):
`src-tauri/src/lib.rs`, `src-tauri/src/model.rs`, `src-tauri/src/error.rs`,
`src/types/model.ts`, `src/api/commands.ts`, `schemas/`, `docs/ARCHITECTURE.md`,
`package.json` / `Cargo.toml` dependency sections (add your deps in a small separate commit).

## 2. Shared types

Source of truth: `schemas/drone-model.schema.json` (DroneModel) and
`src-tauri/src/model.rs` (everything). JSON uses camelCase fields and
SCREAMING_SNAKE_CASE enum values.

### 2.1 DroneModel (wizard answers, firmware-independent)

```jsonc
{
  "schemaVersion": 1,
  "fc":       { "boardName": "SPEEDYBEEF405V4", "firmwareVersion": "4.5.2", "apiVersion": "1.46" },
  "receiver": { "kind": "SERIAL", "protocol": "CRSF", "port": "UART2", "telemetry": true,
                "inverted": false, "halfDuplex": false },
  "vtx":      { "kind": "DIGITAL", "protocol": "MSP", "port": "UART1", "digitalSystem": "DJI", "region": null },
  "gps":      { "enabled": true, "port": "UART4", "provider": "UBLOX" },
  "esc":      { "protocol": "DSHOT600", "bidirectionalDshot": true, "motorPoles": 14,
                "telemetryPort": null, "propsOut": true },
  "switches": [ { "mode": "ARM", "auxChannel": 0, "rangeMin": 1800, "rangeMax": 2100 } ],
  "extras":   { "beeper": null, "ledStrip": null, "craftName": "QUAD" }
}
```

Enums:
- `receiver.kind`: NONE | SERIAL | SPI | PPM | MSP
- `receiver.protocol`: CRSF | GHST | SBUS | FPORT | IBUS | SRXL2 | SPEK1024 | SPEK2048 | SUMD (serial); EXPRESSLRS | FRSKY_D | FRSKY_X | FRSKY_X_LBT (SPI)
- `vtx.kind`: NONE | ANALOG | DIGITAL; `vtx.protocol`: SMARTAUDIO | TRAMP | MSP; `vtx.digitalSystem`: DJI | WALKSNAIL | HDZERO | OTHER
- `gps.provider`: UBLOX | NMEA
- `esc.protocol`: DSHOT150 | DSHOT300 | DSHOT600 | ONESHOT125 | MULTISHOT | PWM
- Port names: `UART1`…`UART15`, `LPUART1`, `SOFTSERIAL1`, `SOFTSERIAL2` (never numeric ids).

### 2.2 Other types (see model.rs for fields)

`HostPort`, `HostPortKind`, `FcInfo`, `FcSerialPort {name, identifier, functionMask, isConnectionPort}`,
`BoardTarget`, `FirmwareRelease`, `FirmwareRequest`, `FirmwareImage`, `FirmwareFormat`,
`DfuDevice`, `FlashOptions`, `FlashProgress`, `FlashStage`, `CliScript {commands, warnings, dialect}`,
`Backup`, `ApplyResult`, `CliError`, `VerifyReport`, `Mismatch`, `DriverStatus`, `AppInfo`.

### 2.3 Errors

Every command returns `Result<T, AppError>`; the UI receives `{ "code": "...", "message": "..." }`.
Codes: NOT_IMPLEMENTED, NOT_CONNECTED, SERIAL, MSP, CLI, USB, FIRMWARE, CONFIG, DRIVER, IO.
The UI maps `code` → i18n key `errors.<code>`.

## 3. Tauri commands

Argument names are camelCase on the JS side (Tauri converts to snake_case Rust params).

| Command | Args | Returns | Owner |
|---|---|---|---|
| `device_list_ports` | – | `HostPort[]` | device |
| `device_connect` | `path` | – | device |
| `device_disconnect` | – | – | device |
| `device_identify` | – | `FcInfo` | device |
| `device_cli_exec` | `line` | `string` | device |
| `device_reboot_to_bootloader` | – | – | device |
| `flasher_list_targets` | – | `BoardTarget[]` | flasher |
| `flasher_list_releases` | `target` | `FirmwareRelease[]` | flasher |
| `flasher_list_dfu_devices` | – | `DfuDevice[]` | flasher |
| `flasher_fetch_firmware` | `request: FirmwareRequest` | `FirmwareImage` | flasher |
| `flasher_list_cache` | – | `FirmwareImage[]` | flasher |
| `flasher_flash` | `image, dfuSerial, options` | – (events) | flasher |
| `config_read_ports` | – | `FcSerialPort[]` | config |
| `config_backup` | – | `Backup` | config |
| `config_generate` | `model, ports` | `CliScript` | config |
| `config_apply` | `script` | `ApplyResult` | config |
| `config_verify` | `model` | `VerifyReport` | config |
| `config_rollback` | `backup` | `ApplyResult` | config |
| `system_driver_status` | – | `DriverStatus` | packaging |
| `system_install_dfu_driver` | – | – | packaging |
| `system_app_info` | – | `AppInfo` | packaging |

Events: `flasher://progress` → `FlashProgress`. New events are named `<module>://<name>`.

## 4. Rust interfaces between modules

```rust
// device/mod.rs — implemented by device, consumed by config and flasher
pub trait Connection: Send {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>>;
    fn enter_cli(&mut self) -> AppResult<()>;
    fn cli_exec(&mut self, line: &str) -> AppResult<String>;
    fn exit_cli(&mut self, save: bool) -> AppResult<()>;
}

// config/mod.rs — pure, golden-tested
pub fn generate(model: &DroneModel, ports: &[FcSerialPort]) -> AppResult<CliScript>;
```

Until `device` lands, `config` tests use a `MockConnection` that replays recorded CLI
transcripts (put fixtures in `src-tauri/tests/fixtures/<firmware>/`).

## 5. Firmware dialects (config)

| Concern | 4.4 / 4.5 | 2025.12+ |
|---|---|---|
| `serial` port identifier | UART1=0 … (n-1); VCP=20; SOFTSERIAL1=30; LPUART1=40 | UART1=51 … (50+n); VCP=20; SOFTSERIAL1=30; LPUART1=40 |
| `save noreboot` | not available | available |
| Function bitmask | same (see `config/serial_functions.rs`) | same |

Always take identifiers from `config_read_ports`; the table is only for tests.

## 6. Flashing sequence (flasher + device + system)

1. `device_identify` → `FcInfo` (board, MCU). If no Betaflight: user picks target from
   `flasher_list_targets` filtered by `DfuDevice.mcuFamily`.
2. `flasher_fetch_firmware` (cache hit or Build API).
3. `config_backup` (if FC runs Betaflight).
4. `device_reboot_to_bootloader` → poll `flasher_list_dfu_devices`; if `driverOk=false` →
   `system_install_dfu_driver` (UAC prompt) → retry.
5. `flasher_flash` (events) → device re-enumerates as VCP → `device_connect` →
   `device_identify` → `config_read_ports`.

## 7. UI structure

- `src/wizard/steps.json` — ordered steps with `when` conditions on DroneModel paths.
- `src/wizard/engine.ts` — `visibleSteps(model)` (pure, tested).
- `src/wizard/registry.ts` — `component` key → Vue component.
- `src/wizard/state.ts` — reactive `{ model, fc, ports, connectedPath }`.
- `src/api/commands.ts` — `CommandMap`, `call()`, `api.*`, `setBackend()`; `src/api/mock.ts`.
- `src/i18n/` — vue-i18n, `he` default + `en`, `applyDirection()` sets `<html dir>`.
  Keys must exist in both locales (enforced by `tests/i18n.test.ts`).

## 8. Packaging & CI

- Tauri bundler targets: NSIS (+ Hebrew/English installer languages) and MSI.
- WinUSB: libwdi-based elevated helper exe shipped as a Tauri sidecar/resource (phase 1),
  invoked by `system_install_dfu_driver`.
- CI (`.github/workflows/build.yml`): on push/PR → windows-latest: `npm ci`, `npm test`,
  `npm run build`, `cargo test`, `tauri build` (artifacts uploaded); ubuntu: `cargo check`.
- Rust toolchain: stable ≥ 1.90 (Tauri 2.12 requirement). Node 22.
