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
| device | `src-tauri/src/device` | `device` | Enumerate COM ports + VID/PID classification (`usb_ids.rs`); open/close serial (115200 8N1); MSP v1/v2 framing, request/response, timeouts; identify FC (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, UID, BUILD_INFO, serial port list, onboard SPI RX detection — §4.1); CLI mode (enter `#`, exec line until prompt, exit/save); reboot to bootloader; reconnect after reboot. Owns the single active connection (`tauri::State`). Implements the `Connection` trait. | Know anything about DroneModel or firmware files. |
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
                "inverted": false, "halfDuplex": false,
                "elrsUid": null, "elrsDomain": null },   // elrs* only for SPI + EXPRESSLRS
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
- `receiver.elrsDomain`: AU433 | AU915 | EU433 | EU868 | IN866 | FCC915 (SX127x) | ISM2400 | CE2400 (SX1280)
- `gps.provider`: UBLOX | NMEA
- `esc.protocol`: DSHOT150 | DSHOT300 | DSHOT600 | ONESHOT125 | MULTISHOT | PWM
- Port names: `UART1`…`UART15`, `LPUART1`, `SOFTSERIAL1`, `SOFTSERIAL2` (never numeric ids).
  The MVP config engine rejects `SOFTSERIAL*` (no soft serial in MVP).
- `receiver.telemetry` defaults to `true`. Per-motor DShot direction reversal is phase 2 and not in the model.

### 2.2 Other types (see model.rs for fields)

`HostPort`, `HostPortKind`, `FcInfo {…, serialPorts: FcSerialPort[], rxSpi: RxSpiInfo}`,
`FcSerialPort {name, identifier, functionMask, isConnectionPort}`,
`RxSpiInfo {present, expresslrsPins, protocol, featureEnabled, resources}`,
`BoardTarget`, `FirmwareRelease`, `FirmwareRequest`, `FirmwareImage`, `FirmwareFormat`,
`DfuDevice`, `FlashOptions`, `FlashProgress`, `FlashStage`, `CliScript {commands, warnings, dialect, cleared}`,
`ClearedPort {port, functions, functionMaskBefore, functionMaskAfter}`, `SerialFunction`,
`Backup {id, path, boardUid, boardName, firmwareVersion, createdAt}`,
`ApplyResult {applied, errors, saved, backup, reconnected}`, `ConfigProgress {stage, fraction, message}`,
`ConfigStage`, `CliError`, `VerifyReport {ok, mismatches}`, `Mismatch {setting, expected, actual}`,
`DriverStatus`, `UsbDriverInfo`, `UsbDeviceKind`, `DriverAction`, `DriverGuidance`, `AppInfo`,
`MotorTestLimits {maxThrottle, watchdogMs}`, `MotorOutputConfig`, `MotorTestSession`,
`MotorTestStopped {reason, error}`, `RcChannels {seq, timestampMs, channels}`, `RcStopped {reason, error}`.

`DriverStatus` (from `system_driver_status`, Windows; all-OK and empty on other OSes):

```jsonc
{
  "dfuWinusb": false,          // a DFU device is plugged in without WinUSB
  "vcp": true,                 // no VCP/bridge device is missing its driver
  "notes": ["STM32 DFU (0483:DF11): no driver bound"],
  "devices": [
    { "vid": 1155, "pid": 57105, "name": "STM32 DFU", "kind": "DFU",
      "instanceId": "USB\\VID_0483&PID_DF11\\205F32...", "service": null,
      "driverOk": false, "action": "INSTALL_DFU_DRIVER", "guidanceIndex": null }
  ],
  "guidance": [
    { "id": "CP210X", "vendor": "Silicon Labs", "chips": "CP210x ...",
      "usbIds": ["10C4:EA60", "10C4:EA61", "10C4:EA62"],
      "downloadUrl": "https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers" }
  ]
}
```

UI rule: `action = INSTALL_DFU_DRIVER` → button calling `system_install_dfu_driver`;
`VENDOR_DOWNLOAD` → show `guidance[guidanceIndex]` (vendor + link, opened in the
system browser); `REPLUG` → ask to replug / check Device Manager. Third-party
bridge drivers are never bundled.

### 2.3 Errors

Every command returns `Result<T, AppError>`; the UI receives
`{ "code": "...", "message": "...", "details"?: {...} }` (`details` only where listed below).
Codes: NOT_IMPLEMENTED, NOT_CONNECTED, SERIAL, MSP, CLI, USB, FIRMWARE, CONFIG,
UNSUPPORTED_FIRMWARE, DRIVER, IO,
plus the device/flasher codes:

| Code | When | `details` | Suggested UI |
|---|---|---|---|
| `BOARD_NOT_FOUND` | Port path gone / FC unplugged | – | "Plug the FC in and pick its port" |
| `PORT_BUSY` | Port held by another program, **or** a second caller inside the app (one connection manager, one owner) — there is no separate BUSY code | `{ port: string, heldBy: "APP" \| "EXTERNAL" }` | `EXTERNAL`: "Close Betaflight App / other tools"; `APP`: wait/retry |
| `DRIVER_MISSING` | DFU device present but cannot be opened (Windows: no WinUSB bound) | `{ vid: number, pid: number, detail: string }` | Offer `system_install_dfu_driver` (owned by packaging), then retry `flasher_list_dfu_devices` |
| `DFU_NOT_FOUND` | No DFU device appeared / requested `dfuSerial` absent | – | Bootloader-button help |
| `READ_PROTECTED` | MCU flash is read-protected; returned by `flasher_flash` **before** anything is erased | – | Explain; offer unprotect (see below) |
| `ERASE_CONFIRMATION_REQUIRED` | `flasher_remove_read_protection` called without `confirmErase: true`; nothing was sent | – | (programming error in the UI) |
| `FLASH_FAILED` | DFU erase/write/leave failed | – | Retry; recovery via DFU |
| `VERIFY_MISMATCH` | Read-back differs from the image | – | Retry flash |
| `MOTOR_TEST_REFUSED` | Motor-test safety gate refused (props not confirmed, throttle outside stop..cap, 3D mode, invalid motor, motors disabled); motors are at stop | – (reason in `message`) | Show the message; nothing spins |
| `BOARD_ARMED` | FC is armed at motor-test start (or became armed) | – | "Disarm first" |
| `MOTOR_TEST_NOT_ACTIVE` | Keepalive after the test stopped (watchdog, armed, disconnect) | – | Reset the motor-test UI |

Generic `SERIAL`/`MSP`/`CLI`/`USB` remain for everything else from device/flasher.
`DRIVER` stays for driver-installation failures (packaging).

**Read protection is never removed automatically.** The UI must state that unprotecting
**erases the whole chip** (firmware and settings) and require an explicit/typed confirmation
before calling `flasher_remove_read_protection { confirmErase: true }`; afterwards the device
re-enumerates and the user flashes again.

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
| `device_probe_board` | – | `FcInfo` (identify + read-only CLI probe, §4.1) | device |
| `device_motor_test_start` | `motorIndex: number\|null, throttle, propsOffConfirmed, limits: MotorTestLimits\|null` | `MotorTestSession` (first call starts, later calls update; §4.2) | device |
| `device_motor_test_keepalive` | – | – (every 200 ms) | device |
| `device_motor_test_stop` | – | – (all motors to stop) | device |
| `device_rc_stream_start` | `rateHz: number\|null` (15, clamped 1–50) | – (events) | device |
| `device_rc_stream_stop` | – | – | device |
| `flasher_list_targets` | – | `BoardTarget[]` | flasher |
| `flasher_list_releases` | `target` | `FirmwareRelease[]` | flasher |
| `flasher_list_dfu_devices` | – | `DfuDevice[]` | flasher |
| `flasher_fetch_firmware` | `request: FirmwareRequest` | `FirmwareImage` | flasher |
| `flasher_list_cache` | – | `FirmwareImage[]` | flasher |
| `flasher_flash` | `image, dfuSerial, options` | – (events); `READ_PROTECTED` if protected | flasher |
| `flasher_remove_read_protection` | `dfuSerial, confirmErase` | – (erases chip, device re-enumerates) | flasher |
| `config_read_ports` | – | `FcSerialPort[]` | config |
| `config_backup` | – | `Backup` | config |
| `config_generate` | `model, ports` | `CliScript` (**preview only**, never applied) | config |
| `config_elrs_uid` | `phrase` | `number[6]` (phrase not stored/logged) | config |
| `config_plan` | `model` | `CliScript` (read-only, see §5.1) | config |
| `config_apply` | `script` | `ApplyResult` (events) | config |
| `config_verify` | `model` | `VerifyReport` | config |
| `config_rollback` | `backup` | `ApplyResult` (events) | config |
| `config_list_backups` | – | `Backup[]` | config |
| `config_backups_dir` | – | `string` | config |
| `config_restore` | `backupId` | `ApplyResult` (events) | config |
| `system_driver_status` | – | `DriverStatus` | packaging |
| `system_install_dfu_driver` | – | – | packaging |
| `system_app_info` | – | `AppInfo` | packaging |

`system_install_dfu_driver` is a no-op returning `Ok` when nothing needs installing
(or on non-Windows); it fails with `DRIVER` if the user declines UAC or install fails.

Events (named `<module>://<name>`):

| Event | Payload |
|---|---|
| `flasher://progress` | `FlashProgress {stage, fraction, message}` |
| `config://progress` | `ConfigProgress {stage, fraction, message}` |
| `device://motor-test-stopped` | `MotorTestStopped {reason: REQUESTED\|WATCHDOG\|ARMED\|DISCONNECTED\|APP_CLOSING\|ERROR, error: AppError\|null}` |
| `device://rc-channels` | `RcChannels {seq, timestampMs, channels: number[]}` |
| `device://rc-stopped` | `RcStopped {reason: REQUESTED\|DISCONNECTED\|ERROR, error: AppError\|null}` |

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

### 4.1 Device contracts

- **One connection manager** (device) owns the single open serial port app-wide. It lives in
  Tauri managed state (`app.manage(device::ConnectionManager::default())` in lib.rs).
  Other modules use it through the state, never by opening ports:
  ```rust
  let mgr = app.state::<device::ConnectionManager>();   // or State<'_, ConnectionManager> in a command
  mgr.with_link(|link: &mut dyn device::FcLink| { link.enter_cli()?; /* … */ link.reconnect(t) })?;
  ```
  The closure holds the port for its whole duration; a concurrent caller gets
  `PORT_BUSY {heldBy: "APP"}` (also while a motor test or RC stream runs). No open port →
  `NOT_CONNECTED`. Another program holding the port → `PORT_BUSY {heldBy: "EXTERNAL"}`.
  Run `with_link` off the async runtime (`tauri::async_runtime::spawn_blocking`).
- **`Connection` + `Reconnect` = `FcLink`** (device/mod.rs). `exit_cli(true)` sends `save` and
  returns once sent; the caller then calls `reconnect(timeout)`, which waits for the port to drop
  and come back (same VID/PID/serial number), reopens it and re-verifies MSP_API_VERSION and
  MSP_UID. Likewise after `exit` on 4.4 (reboots). `exit_cli(false)` uses `exit noreboot` on 4.5+.
- **`FcInfo.uid`**: MSP_UID (160) = 3 × u32 LE, each `toString(16)` without padding,
  concatenated (same as Betaflight Configurator) → backup folder `<boardUid>`.
- **Serial port list** (`FcInfo.serialPorts`): read by `device_identify` with
  `MSP2_COMMON_SERIAL_CONFIG` (0x1009), falling back to `MSP_CF_SERIAL_CONFIG` (54).
  `identifier` is exactly what the board sent (never remapped); `name` is derived from the
  firmware enum (§5 table; 2025.12 also has UART0=50 and PIOUART0..9=70..79; unknown ids →
  `PORT<id>`). Writes must use `identifier`. An invalid serial layout makes the firmware reset
  all ports at boot, so any writer must validate first (config's job).
  `device::serial_ports` exposes the MSP/CLI parsers for config (C2) to reuse.
- **Onboard SPI RX** (`FcInfo.rxSpi`): MSP cannot say whether the board *has* an SPI receiver
  (a build without RX_SPI still reports protocol 0), so `device_identify` fills only hints
  (`featureEnabled`, `protocol` from MSP_RX_CONFIG) and leaves `present: null`.
  `device_probe_board` runs the read-only CLI commands `serial`, `resource`,
  `get rx_spi_protocol` and sets `present` = `RX_SPI_CS` assigned, `expresslrsPins` =
  `RX_SPI_EXPRESSLRS_RESET` + `_BUSY` assigned, `protocol` = value printed by `get`.
  It leaves the CLI with `exit noreboot` on 4.5+; on 4.4 leaving the CLI reboots the FC
  (the manager reconnects) — prefer running it where a reboot is expected anyway.

### 4.2 Motor test safety (device)

- Runs on the manager's connection; while it runs the port is lent to the motor-test worker
  thread, so other device/config/flasher calls get `PORT_BUSY {heldBy: "APP"}` until
  `device_motor_test_stop` (or `device_disconnect`, which stops first).
- **Gate** (checked on every start/update, in order): `propsOffConfirmed`, FC disarmed
  (`MSP_STATUS_EX` bit 0), no 3D feature, motor protocol not DISABLED and motorCount > 0,
  `motorIndex < motorCount`, `stopValue <= throttle <= maxThrottle`. Cap 1150 by default, hard cap
  1250; over-cap requests are **refused, not clamped**. Any refusal puts all motors at stop.
- **Watchdog**: no keepalive within `watchdogMs` (default 500, clamped 100–2000) → stop, event
  `WATCHDOG`. The FC is re-checked every 250 ms; armed → stop, event `ARMED`.
- **Stop paths** — every one sends `MSP_SET_MOTOR` with the stop value for **all 8 motors**
  (twice; MSP_SET_MOTOR has no reply): `device_motor_test_stop`, a gate refusal, watchdog,
  armed, `device_disconnect`, dropping the worker/handle, and the **emergency stop in the Tauri
  run loop**: `on_window_event` on `CloseRequested` and `Destroyed`, and the `App::run` callback on
  `RunEvent::ExitRequested` and `RunEvent::Exit` call `device::emergency_stop_all()` (event
  `APP_CLOSING`). This does not depend on the UI being alive, and it never waits on the
  manager lock (a keepalive may hold it): it sets the test's halt flag, which the worker checks
  before every `MSP_SET_MOTOR`, on keepalive/update and every ≤20 ms, and writes the all-stop
  frame directly through a duplicated port handle (`SerialLink::try_clone_writer`). Port cleanup
  is a single `try_lock`; if the manager is busy the worker stops the motors itself as soon as
  the in-flight exchange returns. After a halt, keepalive/update get `MOTOR_TEST_NOT_ACTIVE`.
- `MSP_SET_ARMING_DISABLED` is **not** sent. Betaflight has no firmware-side MSP motor timeout:
  if USB drops mid-test the last value stays applied, so the UI must tell the user to test on
  USB power with the battery disconnected and props off.
- The RC stream (`device_rc_stream_start`) also lends the port to a worker; it polls `MSP_RC`
  at `rateHz` and ends with `device://rc-stopped`. Motor test and RC stream are mutually
  exclusive (the second gets `PORT_BUSY {heldBy: "APP"}`).

Until `device` lands, `config` tests use a `MockConnection` that replays recorded CLI
transcripts (put fixtures in `src-tauri/tests/fixtures/<firmware>/`).

## 5. Firmware dialects (config)

| Concern | 4.4 / 4.5 | 2025.12+ |
|---|---|---|
| `serial` port identifier | UART1=0 … (n-1); VCP=20; SOFTSERIAL1=30; LPUART1=40 | UART1=51 … (50+n); VCP=20; SOFTSERIAL1=30; LPUART1=40 |
| `save noreboot` | 4.4: no; 4.5: from 4.5.4 (backport PR #15151) | available |
| ESC idle setting | `dshot_idle_value` | `motor_idle` (PR #13936) |
| `feature -RX_MSP` | accepted | no CLI name (omit) |
| `expresslrs_rate_index` / `_switch_mode` | exist (never written) | removed (PR #12936) |
| Function bitmask | same (see `config/serial_functions.rs`) | same |

Always take identifiers from `config_read_ports`; the table is only for tests.
Supported lines: **4.4.x, 4.5.x, 2025.12.x**. Everything else (≤ 4.3, 2026.6.x and newer)
fails with `UNSUPPORTED_FIRMWARE` before any write.

### 5.1 Config engine contract

- **`config_plan(model)`** is read-only. It reads `version`, `serial` and `dump all`, applies the firmware gate,
  SPI ELRS detection, aux-slot allocation and setting availability, and returns the `CliScript`.
  `cleared` lists the managed functions (RX_SERIAL, GPS, ESC_SENSOR, VTX_SMARTAUDIO, VTX_TRAMP,
  VTX_MSP) removed from ports that no longer need them. MSP is also removed from an old MSP
  DisplayPort port only when that port had exactly MSP|VTX_MSP. All other serial functions are
  never touched. The UI shows `cleared` before applying. **The script returned by `config_plan` is the one
  passed to `config_apply`.** `config_generate` is the pure core (ports supplied by the caller,
  default bauds, all aux slots assumed free, no SPI hardware check): **preview only**, also used by
  golden tests; its output is never applied.
- **`config_apply(script)`** runs these stages: `BACKUP` (`diff all` saved to a file) → `SENDING` (`batch start` +
  commands, stop at the first `###ERROR###`) → `SAVING` (`save` while the batch is still active,
  so firmware refuses to save after any error) → `REBOOTING` → `RECONNECTED`. It **resolves only
  after reconnect** and emits `config://progress` at every stage. On a rejected command the FC is
  rebooted without saving (unchanged) and `ApplyResult.errors` is non-empty with `saved=false`.
- **`config_verify(model)`** reads back `serial`, `feature`, `aux` and `get`, and compares only the keys the
  plan writes. It returns `{ok, mismatches:[{setting, expected, actual}]}`.
- **`config_rollback(backup)` / `config_restore(backupId)`** take a safety backup first, then replay the
  backup's `diff all` (it contains `defaults nosave`), drop its `batch end`, then `save`, reboot and reconnect. They emit the same events.
  Both refuse if the backup's firmware line (4.4 / 4.5 / 2025.12) differs from the FC's.
- **Backups** are stored at `<app_data_dir>/backups/<boardUid>/<UTC YYYYMMDDTHHMMSSZ>.txt` (on Windows
  `%APPDATA%\<identifier>\backups\…`) and are kept forever. `config_backups_dir` returns the root
  for the UI's "open backups folder" action.
- **Receiver:** `SERIAL` → `feature RX_SERIAL`, `feature -RX_SPI`, serialrx settings. `SPI`+`EXPRESSLRS`
  → `feature RX_SPI`, `feature -RX_SERIAL`, `set rx_spi_protocol = EXPRESSLRS`, optional
  `expresslrs_uid` (from `receiver.elrsUid`) and `expresslrs_domain`, and no UART. The old receiver UART is cleared.
  The firmware keeps only one RX feature (config.c: PPM > MSP > SERIAL > SPI > PWM), so the
  other feature is always disabled explicitly. SPI ELRS is offered only when `dump all` shows
  `resource RX_SPI_CS` assigned (SX127x boards have no BUSY pin) and `rx_spi_protocol` and
  `expresslrs_uid` exist. Other SPI protocols → `CONFIG` error in MVP.
- **ELRS binding phrase:** only an input. The UI calls `config_elrs_uid(phrase)` and stores
  the returned 6-byte UID in `receiver.elrsUid`; the phrase is never part of the model and is
  never written to disk (models, backups, logs) or logged. Backups contain only `expresslrs_uid`.
- **Telemetry:** `receiver.telemetry=true` with CRSF, FPORT or SPI EXPRESSLRS → `feature TELEMETRY`.
  Otherwise the feature is left untouched.
- **Aux:** only wizard-managed modes are replaced (freed slots are reused first, then free slots). All other
  aux slots are kept.
- **GPS:** provider, auto-config, Galileo and SBAS only. The u-blox model and `gps_auto_baud` stay at defaults.
- **Out of MVP:** soft serial, per-motor DShot reversal (phase 2).

## 6. Flashing sequence (flasher + device + system)

1. `device_identify` → `FcInfo` (board, MCU). If no Betaflight: user picks target from
   `flasher_list_targets` filtered by `DfuDevice.mcuFamily`.
2. `flasher_fetch_firmware` (cache hit or Build API).
3. `config_backup` (if FC runs Betaflight).
4. `device_reboot_to_bootloader` → poll `flasher_list_dfu_devices`; if `driverOk=false` or a
   flasher call fails with `DRIVER_MISSING {vid, pid}` → `system_install_dfu_driver` (UAC
   prompt, packaging) → retry.
5. `flasher_flash` (events). If it fails with `READ_PROTECTED` → confirmation screen ("this
   erases the chip") → `flasher_remove_read_protection {confirmErase: true}` → device
   re-enumerates → back to step 4.
6. After flashing → device re-enumerates as VCP → `device_connect` →
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
- WinUSB: libwdi-based elevated helper exe shipped as a Tauri resource (phase 1),
  invoked by `system_install_dfu_driver`: binds WinUSB only to connected DFU devices that
  lack it (one UAC prompt; the app itself never runs elevated). Details, build and
  LGPL-3.0 obligations: `docs/PACKAGING.md`.
  device/flasher never install drivers; they report `DRIVER_MISSING` with the device's
  `{vid, pid}` (§2.3).
- CI (`.github/workflows/build.yml`): on push/PR → windows-latest: `npm ci`, `npm test`,
  `npm run build`, `cargo test`, `tauri build` (artifacts uploaded); ubuntu: `cargo check`.
- Rust toolchain: stable ≥ 1.90 (Tauri 2.12 requirement). Node 22.
