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

Optional fields (all 4.4 / 4.5 / 2025.12 / 2026.6; `null` or absent = **keep the FC's current value**, nothing
is written; the generator rejects out-of-range values with `CONFIG`, nothing sent):

| Field | CLI | Rule |
|---|---|---|
| `extras.craftName` | `set craft_name = …` | 1–16 chars of printable ASCII (0x20–0x7E) except `#` and `?`, no leading/trailing space, not exactly `-`; `""` clears it (written as `-`). One rule: schema `pattern` = `model::CRAFT_NAME_PATTERN` = `CRAFT_NAME_PATTERN` in model.ts (tests compare them over `schemas/fixtures/craft-name-cases.json`). Why: craft_name is MODE_STRING 1..16 (`MAX_NAME_LENGTH`); the CLI only buffers 0x20–0x7E, cuts the line at `#` (comment), treats `?` as help in the interactive CLI that MSP opens, strips trailing spaces and skips leading ones, and reads exactly `-` as empty. |
| `vtx.table` | `vtxtable …` | ≤ 8 bands, 8 channels, 8 power levels; band name ≤ 8 chars, power label ≤ 3; frequencies ≤ 5999 MHz (0 = unused); same channel count for every band (`drivers/vtx_table.h`). `null` keeps the FC's table. |
| `vtx.band`, `vtx.channel` | `vtx_band`, `vtx_channel` | 1–8, 1-based into the table (written or the FC's). |
| `vtx.power` | `vtx_power` | **1–7**, 1-based into `table.powerLevels` (0 = unknown, never written). The CLI range is `0..VTX_TABLE_MAX_POWER_LEVELS-1`, so the 8th level of an 8-level table can't be selected with the CLI. It must also be ≤ the table's level count. |
| `esc.idle` | 2025.12+ `motor_idle`; 4.4/4.5 `dshot_idle_value` | 0–2000 in 0.01 % (550 = 5.5 %). On 4.4/4.5 it is DShot-only (warning, not written, for analog protocols). |
| `esc.motorOrder` | `motor_output_reordering` | Item *i* = 0-based output for motor *i+1*; must be a permutation of `0..n` (n ≤ the FC's list length, 8). Remaining slots keep identity. |
| `gps.galileo` | `gps_ublox_use_galileo` | UBLOX only (warning, not written, for NMEA). |
| `gps.sbasMode` | `gps_sbas_mode` | AUTO \| EGNOS \| WAAS \| MSAS \| GAGAN \| NONE. |

The `gps.*` fields only apply when `gps.enabled`.

#### 2.1.1 Model batch 2: modes, beepers, sensors, board alignment, OSD (shape; implementation follows)

All new fields are optional and mean the same on 4.4 / 4.5 / 2025.12 / 2026.6 unless noted. **null or absent = keep the FC's current state**; nothing is written for them. Sources: Betaflight `src/main` at tags 4.4.3, 4.5.5, 2025.12.5 and 2026.6.2.

```jsonc
{
  "switches": [ { "mode": "AIR MODE", "auxChannel": 1, "rangeMin": 1700, "rangeMax": 2100 },
                { "mode": "FLIP OVER AFTER CRASH", "auxChannel": 2, "rangeMin": 1700, "rangeMax": 2100 },
                { "mode": "PREARM", "auxChannel": 3, "rangeMin": 1700, "rangeMax": 2100 } ],
  "airmode": "SWITCH",                       // ALWAYS | SWITCH | OFF | null
  "removeModes": [],                         // e.g. ["PREARM"]: delete that optional mode's FC ranges
  "beepers": { "disabled": ["ON_USB"], "dshotBeaconDisabled": ["RX_SET"] },
  "sensors": { "acc": true, "baro": false, "mag": null },
  "boardAlignment": { "roll": 0, "pitch": 0, "yaw": 90 },
  "osd": { "videoSystem": "HD",
           "elements": [ { "name": "MAIN_BATT_VOLTAGE", "enabled": true, "x": 1, "y": 18 },
                         { "name": "LINK_QUALITY", "enabled": true },        // x/y null = default layout
                         { "name": "RSSI_VALUE", "enabled": false } ] }
}
```

- **Optional modes.** The stable names follow `config::modes` (firmware box names): `PREARM` (permanent id 36), `FLIP OVER AFTER CRASH` (35, "turtle"), `AIR MODE` (28). Aliases like `AIRMODE` and `TURTLE` are also accepted, but the UI writes the canonical names. These three are managed **only when the model mentions them**:
  - an entry in `switches` replaces that mode's FC ranges;
  - a name in `removeModes` deletes them;
  - otherwise the FC's ranges are kept.

  The always-managed modes (ARM, ANGLE, …) keep today's rule: their FC ranges are replaced by `switches`.
- **`airmode`.** On every supported line, air mode is `feature AIRMODE || box AIRMODE` (`fc/rc_modes.c`). The AIR MODE box is only offered when the feature is off (`msp/msp_box.c`). The feature is on by default (`config/feature.c`). 2025.12 renamed `airmodeIsEnabled()` to `isAirmodeEnabled()` but did not change the behaviour.
  - `ALWAYS`: `feature AIRMODE`. Any AIR MODE range is removed, since it would be dead.
  - `SWITCH`: `feature -AIRMODE` plus the `switches` entry. If that entry is missing, it's a `CONFIG` error.
  - `OFF`: `feature -AIRMODE`, and AIR MODE ranges are removed.
  - Rejected with `CONFIG`: an AIR MODE switch together with `ALWAYS` or `OFF`.
- **`beepers`.** The stable names are the 4.4/4.5 CLI names from `beeper list` (`io/beeper.c` `beeperTable`). When `beepers` is set, the generator writes every condition the firmware has: `beeper -X` for the listed ones and `beeper X` for the rest. This gives a full, verifiable state. Per-version differences:
  - On 2025.12, `CRASH_FLIP` is written as `CRASHFLIP`.
  - `RC_SMOOTHING_INIT_FAIL` doesn't exist on 2025.12. Listing it there gives a warning and nothing is written for it.
  - 2026.6 has the same beeper table as 2025.12.
  - The CLI matches names by prefix (`strncasecmp`), so the generator always writes full names.
  - Firmware default: all conditions on (`beeper_off_flags` = 0). The DShot beacon is off for both RX_LOST and RX_SET (`DEFAULT_DSHOT_BEACON_OFF_FLAGS`).
  - `dshotBeaconDisabled` maps to `beacon -X` / `beacon X` for the only allowed conditions, RX_LOST and RX_SET.
  - `beeper` and `beacon` become guard-allowlisted verbs in the implementation PR.
  - The old `extras.beeper` boolean is not used by the generator. It stays accepted, is ignored, and is superseded by `beepers`.
- **`sensors`.**
  - `true` sets `acc_hardware` / `baro_hardware` / `mag_hardware` to `AUTO`. If the FC already names a specific chip, that chip is kept.
  - `false` sets it to `NONE`. No `feature` is involved on 4.4+.
  - The gyro is always on.
  - With ACC off, the firmware hides ANGLE, HORIZON, HEADFREE and ACRO TRAINER (plus ALTHOLD and POSHOLD on 2025.12). Failsafe and GPS rescue can't self-level, because angle mode requires `sensors(SENSOR_ACC)` (`fc/core.c`). So `acc: false` together with an ANGLE, HORIZON, ALTHOLD, POSHOLD or GPS RESCUE switch is a `CONFIG` error.
  - `baro: false` with ALTHOLD gives a warning: ALTHOLD can fall back to GPS altitude.
  - Turning ACC on needs an accelerometer calibration before arming (Device's `device_acc_calibrate`).
  - Device's `SensorState.configuredHardware` reads the same settings: `NONE` = `false` (disabled), anything else = `true`.
  - **Calibration is never interleaved with a config batch.** The firmware saves the whole config to flash when an ACC calibration finishes. So the UI runs `device_acc_calibrate` only after `config_apply` has returned (saved, rebooted, reconnected), or before `config_plan`, never between them. The config script doesn't rely on unsaved RAM state outside its own batch: every batch ends in `save` + reboot (or `exit`), and the batch holds the port, so a calibration can't start mid-batch (`PORT_BUSY`).
- **`boardAlignment`.** Maps to `align_board_roll` / `_pitch` / `_yaw`, in whole degrees, range −180..360 on all three lines (`cli/settings.c`). These are the same values that `MSP_BOARD_ALIGNMENT_CONFIG` reports. That message sends them as U16, so Device's `device_sensors()` must sign-extend them to int16. Device reports them as `SensorAlignment.boardRollDeg` / `boardPitchDeg` / `boardYawDeg` (signed whole degrees, #63), which are the same values as `boardAlignment.roll` / `pitch` / `yaw`.
- **`osd`.**
  - **Setting `osd` writes:**
    - always: `feature OSD`;
    - when `videoSystem` is set: `vcd_video_system`;
    - for HD: `osd_displayport_device = MSP` and `osd_canvas_width = 53` / `osd_canvas_height = 20`. The CLI doesn't resize the canvas; only MSP_SET_OSD_CONFIG does that (`msp/msp.c`).
    - for analog: `osd_displayport_device = AUTO`.
  - **Grids:** PAL 30×16, NTSC 30×13, AUTO 30×13 (fits either camera), HD 53×20 (`drivers/display.h`, `osd/osd.h`).
  - **Elements:**
    - Each element maps to `osd_<cli>_pos`. The stable name is the `osd_items_e` name without `OSD_`, e.g. `MAIN_BATT_VOLTAGE` → `osd_vbat_pos`, `ITEM_TIMER_2` → `osd_tim_2_pos`.
    - The 4.4 list has 77 elements; 4.5 adds `GPS_LAP_TIME_*`; 2025.12 adds `DEBUG2` and `LIDAR_DIST`; 2026.6 adds `POS_HOLD_READY`, `BATTERY_PROFILE_NAME`, `CUSTOM_SERIAL_TEXT`, `WP_*` (8) and `NAV_MAP` (94). An unknown name for the firmware is a `CONFIG` error.
    - Elements not listed keep the FC's state.
    - An element whose x/y doesn't fit the grid is a `CONFIG` error.
  - **Position encoding** is identical on all three lines (`osd/osd.h`):
    - x bits 0–4 plus bit 10 (x ≤ 63);
    - y bits 5–9;
    - bits 11–13: visible in OSD profile 1/2/3;
    - bits 14–15: element variant (type).

    `enabled` sets or clears all three profile bits. The type bits are kept from the FC.
  - **Catalog:** `config_firmware_catalog(firmwareVersion)` lists, per firmware line, the beeper conditions, DShot beacon events, OSD element names and stable mode names, so the UI doesn't hard-code them. It describes the line, not one board's build; plan checks the FC's `dump all`.
  - **Defaults:** config owns the default positions. The firmware's own defaults put every element hidden at the centre, so they aren't useful. `config_osd_default_layout(videoSystem, firmwareVersion)` returns `OsdLayout {videoSystem, cols, rows, elements: {name, x, y, enabled}[]}` for that firmware line. The UI preview and the generator use the same table.

### 2.2 Other types (see model.rs for fields)

`HostPort`, `HostPortKind`, `FcInfo {…, buildOptions: string[], hdOsdSupported: boolean|null, serialPorts: FcSerialPort[], rxSpi: RxSpiInfo}`,
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
`MotorTestStopped {reason, error}`, `RcChannels {seq, timestampMs, channels}`, `RcStopped {reason, error}`,
`SensorStatus {gyro, acc, baro, mag: SensorState, accCalibrated, alignment}`,
`SensorState {present, enabled, configuredHardware, detectedHardware}`,
`SensorAlignment {boardRollDeg, boardPitchDeg, boardYawDeg, gyroAlignment}`,
`AccCalibrationResult {durationMs, completion: CALIBRATING_FLAG|FIXED_WAIT, maxGyroRateDps}`,
`Attitude {seq, timestampMs, roll, pitch, yaw}`, `AttitudeStopped {reason, error}` (§4.3).

`DriverStatus` (from `system_driver_status`, Windows; all-OK and empty on other OSes):

```jsonc
{
  "dfuWinusb": false,          // a DFU device is plugged in without WinUSB
  "vcp": true,                 // no VCP/bridge device is missing its driver
  "notes": ["STM32 DFU (0483:DF11) ...: service=STTub30, problem=0",
            "STM32 DFU (0483:DF11): installing WinUSB replaces driver STTub30"],
  "devices": [
    { "vid": 1155, "pid": 57105, "name": "STM32 DFU", "kind": "DFU",
      "instanceId": "USB\\VID_0483&PID_DF11\\205F32...", "service": "STTub30",
      "driverOk": false, "action": "INSTALL_DFU_DRIVER", "guidanceIndex": null,
      "replacesDriver": true }   // another driver is bound and will be replaced
  ],
  "guidance": [
    { "id": "CP210X", "vendor": "Silicon Labs", "chips": "CP210x ...",
      "usbIds": ["10C4:EA60", "10C4:EA61", "10C4:EA62"],
      "downloadUrl": "https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers" }
  ]
}
```

UI rule: `action = INSTALL_DFU_DRIVER` → button calling `system_install_dfu_driver`;
if `replacesDriver` is true, first warn that the current driver (`service`, e.g. ST's
DfuSe `STTub30`) will be replaced by WinUSB (tools that rely on it, such as DfuSe,
stop seeing the board) and require confirmation;
`VENDOR_DOWNLOAD` → show `guidance[guidanceIndex]` (vendor + link, opened in the
system browser); `REPLUG` → ask to replug / check Device Manager. Third-party
bridge drivers are never bundled.

### 2.3 Errors

Every command returns `Result<T, AppError>`; the UI receives
`{ "code": "...", "message": "...", "details"?: {...} }` (`details` only where listed below).
Codes: NOT_IMPLEMENTED, NOT_CONNECTED, SERIAL, MSP, CLI, USB, FIRMWARE, CONFIG,
UNSUPPORTED_FIRMWARE, DRIVER, IO, INVALID_ARGUMENT (bad command argument, see the table),
plus the device/flasher codes:

| Code | When | `details` | Suggested UI |
|---|---|---|---|
| `BOARD_NOT_FOUND` | Port path gone / FC unplugged | – | "Plug the FC in and pick its port" |
| `PORT_BUSY` | Port held by another program, **or** a second caller inside the app (one connection manager, one owner) — there is no separate BUSY code | `{ port: string, heldBy: "APP" \| "EXTERNAL" }` | `EXTERNAL`: "Close Betaflight App / other tools"; `APP`: wait/retry |
| `CONFIG` | The model can't be turned into a script for this FC (`config_generate`, `config_plan`, `config_verify`); nothing is sent. Usually no `details`. | `{ reason: "OSD_HD_NO_DISPLAYPORT" }` only when `osd.videoSystem = HD` but the FC's `dump all` has no MSP DisplayPort (`displayport_msp_*` missing), on all three firmware lines. This comes from `config_plan` / `config_verify`, which read the dump. `config_generate` has no dump, so it never raises it. | `OSD_HD_NO_DISPLAYPORT`: back to the OSD step (pick analog or flash firmware with HD OSD); otherwise show the message |
| `DRIVER_MISSING` | DFU device present but cannot be opened (Windows: no WinUSB bound) | `{ vid: number, pid: number, detail: string }` | Offer `system_install_dfu_driver` (owned by packaging), then retry `flasher_list_dfu_devices` |
| `DFU_NOT_FOUND` | No DFU device appeared / requested `dfuSerial` absent | – | Bootloader-button help |
| `READ_PROTECTED` | MCU flash is read-protected; returned by `flasher_flash` **before** anything is erased | – | Explain; offer unprotect (see below) |
| `ERASE_CONFIRMATION_REQUIRED` | `flasher_remove_read_protection` called without `confirmErase: true`; nothing was sent | – | (programming error in the UI) |
| `INVALID_ARGUMENT` | A command argument is invalid, e.g. `flasher_flash` given an image path that is not a file inside the firmware cache (canonicalized; `..` and symlink escapes rejected); nothing was sent | – | (programming error in the UI) |
| `BUILD_OPTIONS_UNAVAILABLE` | `flasher_fetch_firmware` / `flasher_required_options`: the Build API option list of the release (`/api/options/{release}`) could not be loaded (network, HTTP status, bad JSON) and no cached copy exists. Nothing was built or flashed: a build without the board's options is never requested (§6 "Build options"). **Retryable** | `{ release: string }` | "Couldn't reach the firmware build service", retry |
| `REQUIRED_OPTION_UNSUPPORTED` | `flasher_fetch_firmware` / `flasher_required_options`: a required option (`FirmwareRequest.requiredOptions`, the wizard's choice, e.g. `USE_OSD_HD`) is not offered by the target release. Nothing was built or flashed | `{ release: string, options: string[] }` | Pick a newer release, or change the choice (e.g. analog OSD) |
| `FLASH_FAILED` | DFU erase/write/leave failed | – | Retry; recovery via DFU |
| `VERIFY_MISMATCH` | Read-back differs from the image | – | Retry flash |
| `MOTOR_TEST_REFUSED` | Motor-test safety gate refused (props not confirmed, throttle outside stop..cap, 3D mode, invalid motor, motors disabled); motors are at stop | – (reason in `message`) | Show the message; nothing spins |
| `BOARD_ARMED` | FC is armed at motor-test start (or became armed), or at `device_acc_calibrate` (nothing sent) | – | "Disarm first" |
| `MOTOR_TEST_NOT_ACTIVE` | Keepalive after the test stopped (watchdog, armed, disconnect) | – | Reset the motor-test UI |
| `ACC_NOT_AVAILABLE` | `device_acc_calibrate`: no accelerometer detected, or disabled (`acc_hardware = NONE`), or firmware built without one; nothing was sent | `{ present: boolean, enabled: boolean }` | `enabled: false`: "The accelerometer is turned off"; else "No accelerometer found" (skip the level step) |
| `BOARD_MOVING` | `device_acc_calibrate`: gyro rate above the limit (§4.3). `during: false`: checked before, nothing was sent. `during: true`: moved while calibrating, the FC already stored the (bad) result | `{ maxRateDps: number, limitDps: number, during: boolean }` | "Put the drone down level and keep it still", then retry |
| `ACC_CALIBRATION_FAILED` | `device_acc_calibrate`: the FC still reports calibrating after 4 s, or still reports "accelerometer needs calibration" afterwards | – | Retry; then suggest a power cycle |
| `PENDING_UNSAVED_CHANGES` | `device_acc_calibrate`: the FC may hold unsaved changes in RAM (§4.1 "RAM state"), e.g. a config apply failed and its cleanup reboot was not confirmed. Calibration saves the whole RAM config to EEPROM, so it is refused (checked first, nothing was sent) | `{ reason: "CLI_COMMAND" \| "EXIT_NOREBOOT" \| "MSP_WRITE" }` | Offer "Reboot board" (`device_reboot`), then retry. A plain reconnect or USB replug does **not** clear it (a battery-powered FC keeps its RAM) |

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
| `device_reboot` | – | – (resolves once the reboot is confirmed; §4.1 "RAM state") | device |
| `device_probe_board` | – | `FcInfo` (identify + read-only CLI probe, §4.1) | device |
| `device_motor_test_start` | `motorIndex: number\|null, throttle, propsOffConfirmed, limits: MotorTestLimits\|null` | `MotorTestSession` (first call starts, later calls update; §4.2) | device |
| `device_motor_test_keepalive` | – | – (every 200 ms) | device |
| `device_motor_test_stop` | – | – (all motors to stop) | device |
| `device_rc_stream_start` | `rateHz: number\|null` (15, clamped 1–50) | – (events) | device |
| `device_rc_stream_stop` | – | – | device |
| `device_sensors` | – | `SensorStatus` (read-only; §4.3) | device |
| `device_acc_calibrate` | – | `AccCalibrationResult`, resolves when the FC is done (§4.3) | device |
| `device_get_attitude` | – | `Attitude` (one sample, `seq` = `timestampMs` = 0) | device |
| `device_attitude_stream_start` | `rateHz: number\|null` (10, clamped 1–30) | – (events) | device |
| `device_attitude_stream_stop` | – | – | device |
| `flasher_list_targets` | – | `BoardTarget[]` | flasher |
| `flasher_list_releases` | `target` | `FirmwareRelease[]`: hides withdrawn releases and any outside `config::firmware::SUPPORTED_LINES` (e.g. 2026.12.0-alpha) | flasher |
| `flasher_list_dfu_devices` | – | `DfuDevice[]` | flasher |
| `flasher_fetch_firmware` | `request: FirmwareRequest` | `FirmwareImage` (`droppedOptions`: board options the release does not offer; §6 "Build options") | flasher |
| `flasher_required_options` | `requirements: FirmwareRequirements, release: string\|null` | `string[]` (Build API option values for `FirmwareRequest.requiredOptions`; read-only, no FC/USB I/O; with a release: `REQUIRED_OPTION_UNSUPPORTED` / `BUILD_OPTIONS_UNAVAILABLE`) | flasher |
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
| `config_osd_default_layout` | `videoSystem: OsdVideoSystem, firmwareVersion: string` | `OsdLayout` (read-only, no FC I/O; `UNSUPPORTED_FIRMWARE` for other lines) | config |
| `config_firmware_catalog` | `firmwareVersion: string` | `FirmwareCatalog {beepers: BeeperName[], beaconEvents: DshotBeaconCondition[], osdElements: string[], modes: string[]}` (read-only, no FC I/O; `UNSUPPORTED_FIRMWARE` for other lines) | config |
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
| `device://attitude` | `Attitude {seq, timestampMs, roll, pitch, yaw}` (degrees, §4.3) |
| `device://attitude-stopped` | `AttitudeStopped {reason: REQUESTED\|DISCONNECTED\|ERROR, error: AppError\|null}` |

## 4. Rust interfaces between modules

```rust
// device/mod.rs — implemented by device, consumed by config and flasher
pub trait Connection: Send {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>>;
    fn enter_cli(&mut self) -> AppResult<()>;
    fn cli_exec(&mut self, line: &str) -> AppResult<String>;
    fn exit_cli(&mut self, save: bool) -> AppResult<()>;
    fn uid(&self) -> Option<String> { None } // cached FcInfo.uid (device implements it)
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
  `uid()` returns the connected board's cached `FcInfo.uid` (no I/O; `None` if unknown).
- **CLI state on the link**: the manager remembers that the CLI is open from `enter_cli`
  until `exit_cli`, `reconnect`, a reboot or an error. Callers may leave it open (config's
  read-only commands do, because on 4.4 leaving reboots the FC). The next MSP call — from
  `msp_request`, `device_identify`, motor test or RC stream — first leaves it transparently:
  plain `exit` (on every firmware, so unsaved RAM changes are dropped; `exit noreboot` is
  never used here), wait for the reboot (if no `Rebooting` line was seen: MSP_REBOOT `[0]`
  when the RAM state is dirty or nothing at all was seen; a port drop alone only suffices
  while RAM is clean, see "RAM state"), wait for the
  port to come back, re-verify MSP_API_VERSION + MSP_UID. One `exit` per transition, `save`
  is never sent. The link is detached before the `exit` is written, so a failure leaves no
  CLI state: the call fails with `SERIAL` (timed out coming back / a different flight
  controller came back) and later calls get `SERIAL` "reconnect first", never a second
  `exit`. `device_reboot_to_bootloader` from an open CLI sends `bl rom` directly. The
  emergency stop never waits for this (it does not take the port lock).
- **RAM state ("dirty RAM")**: the connection tracks whether the FC's RAM may hold changes
  that are not in EEPROM, because some FC operations save the *whole* RAM config (accelerometer
  calibration calls `saveConfigAndNotify()`), which would persist a half-applied config.
  Tracked entirely inside the device link; callers need no extra calls.
  - **Set** (first reason kept) when, through the link: `cli_exec` sends a line that is not
    read-only (`CLI_COMMAND`; read-only = empty / `#` comment, `version`, `status`, `tasks`,
    `help`, `dump …`, `diff …`, `get …`, and listing commands without arguments such as
    `serial`, `feature`, `aux`, `resource`; `batch start` counts as a change); `exit_cli(false)`
    leaves with `exit noreboot` on 4.5+ (`EXIT_NOREBOOT`); `msp_request` sends any code that
    is not a known read (`MSP_WRITE`: every `MSP_SET_*`, `MSP_EEPROM_WRITE`, `MSP_RESET_CONF`,
    `MSP_SELECT_SETTING`, `MSP_COPY_PROFILE`, unknown codes, …). `MSP_REBOOT` itself does not
    set it. The flag is set before the line/frame is written, so a failed send still counts.
  - **Cleared only by a confirmed reboot**: a reboot **the app issued** in the same sequence
    whose effect is acknowledged by the FC, i.e. it answered `MSP_REBOOT`, or printed
    `Rebooting` after our `exit` / `save`, **and** the port then came back (or reopened, UART
    bridge) with MSP_API_VERSION + MSP_UID re-verified as the same board. Where that happens:
    `reconnect` after an acknowledged reboot (config's save / discard), the CLI exit of the
    previous bullet, `device_reboot`, the connect-time reboot below. A change after the
    acknowledgement voids it.
  - **Never cleared by a port drop or reconnect alone.** With the battery connected a USB
    unplug/replug drops the port but the FC keeps its RAM, so a port drop, a plain
    `reconnect`, or a `device_connect` is never proof of a reboot. In particular: an
    `MSP_REBOOT` answered with an error frame (or not answered) followed by a port drop does
    not clear it, and after our `exit` a port drop without `Rebooting` does not count (the
    link then sends `MSP_REBOOT [0]` and needs its reply). `save` alone does not clear it;
    `save` printing `Rebooting` + reconnect does.
  - **Boot evidence is not used.** No MSP message carries an uptime or boot counter on 4.4,
    4.5 or 2025.12 (`msp.c` has no `millis()`; `MSP_STATUS(_EX)` / `MSP_ANALOG` carry cycle
    time, sensors, flags, battery). Uptime only exists in CLI `status` ("System Uptime"), and
    reading it needs the CLI, which on 4.4 can only be left by rebooting.
  - **`device_reboot`** ("Reboot board"): leaves an open CLI with plain `exit` (never `save`),
    otherwise sends `MSP_REBOOT [0]`; waits for the port to drop and return (UART bridge:
    reopens), re-verifies MSP_API_VERSION + MSP_UID, clears the flag, resolves. Betaflight
    replies to `MSP_REBOOT` before resetting (`msp.c` sets `mspRebootFn` as post-process;
    `msp_serial.c` waits for the TX to finish before running it). Errors: not confirmed (no
    reply / error frame / no `Rebooting`) but the same board answered →
    `PENDING_UNSAVED_CHANGES {reason}` if the flag is set (still set), else `MSP`; the board
    did not come back in time or a different board came back → `SERIAL`; also
    `NOT_CONNECTED`, `PORT_BUSY`.
  - **Fresh connect**: a new connection starts clean, except for a board that was dirty when
    this app last closed its port (`device_disconnect`, `device_connect` to another port, port
    lost; matched by MSP_UID, else the path): `device_connect` then issues the same
    confirmed reboot once before returning. `device_connect` to the already-open port reopens
    it (same reboot) while the flag is set, instead of returning early. If that reboot is not
    confirmed, `device_connect` fails (`PENDING_UNSAVED_CHANGES` / `MSP` / `SERIAL`, as
    `device_reboot`) and the board stays marked.
- **`FcInfo.uid`**: exactly the CLI `mcu_id` string: 24 lowercase hex chars, the three
  MSP_UID (160) words `U_ID_0, U_ID_1, U_ID_2` (each u32 little-endian on the wire), each
  zero-padded to 8 digits, in that order (`cli.c`: `"%08x%08x%08x", U_ID_0, U_ID_1, U_ID_2`).
  Example: payload `2b 00 33 00 0b 51 38 31 33 37 38 32` → `0033002b3138510b32383733`.
  Fixed width, so it is unambiguous as a key (Configurator's unpadded `toString(16)` form is
  not; it is not used). → backup folder `<boardUid>`; reconnect compares the raw words.
- **`FcInfo.mcu`** (display only). On 4.4 to 2025.12 it is the `mcuTypeNames` entry for the
  MSP_BOARD_INFO mcu type id (e.g. `F40X`). 2026.6 always sends id 254
  (`MCU_TYPE_ID_PROVIDED_BY_NAME`), so `device_identify` then reads `MSP2_MCU_INFO` (0x300C:
  u8 id + pstring) and reports that name (e.g. `STM32F405`, `SIMULATOR` on SITL). It is best
  effort: `null` if the request fails.
- **Serial port list** (`FcInfo.serialPorts`): read by `device_identify` with
  `MSP2_COMMON_SERIAL_CONFIG` (0x1009), falling back to `MSP_CF_SERIAL_CONFIG` (54).
  `identifier` is exactly what the board sent (never remapped); `name` is derived from the
  firmware enum (§5 table; 2025.12+ also has UART0=50 and PIOUART0..9=70..79; unknown ids →
  `PORT<id>`). Writes must use `identifier`. An invalid serial layout makes the firmware reset
  all ports at boot, so any writer must validate first (config's job).
  `device::serial_ports` exposes the MSP/CLI parsers for config (C2) to reuse.
- **Onboard SPI RX** (`FcInfo.rxSpi`): MSP cannot say whether the board *has* an SPI receiver
  (a build without RX_SPI still reports protocol 0), so `device_identify` fills only hints
  (`featureEnabled`, `protocol` from MSP_RX_CONFIG) and leaves `present: null`.
  `device_probe_board` runs the read-only CLI commands `serial`, `resource`,
  `get rx_spi_protocol` (and `get displayport_msp_col_adjust`, see "HD OSD support") and sets `present` = `RX_SPI_CS` assigned, `expresslrsPins` =
  `RX_SPI_EXPRESSLRS_RESET` + `_BUSY` assigned, `protocol` = value printed by `get`.
  It leaves the CLI with `exit noreboot` on 4.5+; on 4.4 leaving the CLI reboots the FC
  (the manager reconnects) — prefer running it where a reboot is expected anyway.
- **Build options** (`FcInfo.buildOptions`): `device_identify` reads MSP_BUILD_INFO (5). From
  API 1.46 (4.5, 2025.12, 2026.6) the reply is `date[11] time[8] gitRev[7]` followed by the compiled-in
  option ids as u16 LE until the end of the payload (firmware `msp/msp_build_info.c`
  `sbufWriteBuildInfoFlags`). Ids are mapped to Build API defines (`msp/msp_build_info.h`,
  e.g. 16416 `USE_OSD_SD`, 16417 `USE_OSD_HD`, 4097 `USE_SERIALRX_CRSF`); unknown ids are
  dropped, and duplicates are listed once (2026.6.2 sends 16427 `USE_FLIGHT_PLAN` twice). 4.4
  (API 1.45) sends only date/time/gitRev → `[]`.
- **HD OSD support** (`FcInfo.hdOsdSupported`): whether the build has `USE_OSD_HD` (without it
  the firmware forces `vcd_video_system` HD → AUTO, `io/displayport_msp.c`). First firm answer wins:
  1. MSP_BUILD_INFO option list (4.5+, non-empty): `true` iff it contains 16417 `USE_OSD_HD`,
     else `false`.
  2. MSP_OSD_CANVAS (189) with an empty reply: the build has no `USE_OSD` at all → `false`
     (any OSD build answers 2 bytes, which says nothing about HD).
  3. `device_probe_board` also runs `get displayport_msp_col_adjust`; `INVALID NAME` means no
     MSP DisplayPort (`USE_MSP_DISPLAYPORT` is defined together with `USE_OSD`) → `false`.
  Otherwise `null` — in practice 4.4 with an OSD build. The setting existing is *not* taken as
  `true`: `USE_MSP_DISPLAYPORT` (and so `displayport_msp_col_adjust`) is also compiled into
  SD-only (`USE_OSD_SD` without `USE_OSD_HD`) builds (`target/common_pre.h`). Config's plan
  rejects HD when `displayport_msp_col_adjust` is missing, which agrees with every `false`
  above; the UI should hide/disable HD on `false` and allow it with a caution on `null`.
  On `null` the UI (`src/wizard/osd.ts` `hdOsdSupport`) falls back to `FcInfo.buildOptions`:
  `USE_OSD_HD` → `true`; any of `USE_OSD` / `USE_OSD_SD` / `USE_MSP_DISPLAYPORT` without it →
  `false` (`USE_MSP_DISPLAYPORT` alone is never HD, see above); no OSD option (or `[]`) → `null`.

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
  if USB drops mid-test the last value stays applied until the FC is power-cycled. The motor test
  needs the battery connected (ESCs need power), so the UI must collect, before setting
  `propsOffConfirmed: true`, explicit confirmation that **props are removed**, the **battery is
  connected**, and **hands are clear**. The backend does not require or check battery state;
  the throttle cap, disarmed check, watchdog and emergency stop above are what limit the risk.
- The RC stream (`device_rc_stream_start`) also lends the port to a worker; it polls `MSP_RC`
  at `rateHz` and ends with `device://rc-stopped`. Motor test and RC stream are mutually
  exclusive (the second gets `PORT_BUSY {heldBy: "APP"}`).

Until `device` lands, `config` tests use a `MockConnection` that replays recorded CLI
transcripts (put fixtures in `src-tauri/tests/fixtures/<firmware>/`).

### 4.3 Sensors, accelerometer calibration and attitude (device)

Read-only except `MSP_ACC_CALIBRATION`. Writing sensor enables (`acc_hardware` …) or the board
/ sensor alignment belongs to the config engine; device sends no such command. Line references
are Betaflight 4.4.3 / 4.5.5 / 2025.12.5.

- **`device_sensors`** — `MSP_API_VERSION`, `MSP_STATUS_EX` (`MSP_STATUS` fallback),
  `MSP_SENSOR_CONFIG` (96), `MSP2_SENSOR_CONFIG_ACTIVE` (0x300A, 4.5+ only), `MSP_BOARD_INFO`,
  `MSP_BOARD_ALIGNMENT_CONFIG` (38), `MSP_SENSOR_ALIGNMENT` (126).
  `present` = the `MSP_STATUS` sensor bit (ACC 0, BARO 1, MAG 2, GYRO 5): detected at boot and
  running; a sensor disabled in the config is never initialised, so it is never present.
  `enabled` = configured hardware ≠ `NONE` (gyro: always true). Hardware names use the
  firmware's lookup tables for the connected API version (they differ between 4.4, 4.5 and
  2025.12). `accCalibrated` = `MSP_BOARD_INFO` configuration problem bit 0
  (`PROBLEM_ACC_NEEDS_CALIBRATION`) clear. Optional parts the firmware does not answer are
  `null`.
- **`device_acc_calibrate`** (holds the port for the whole run; others get `PORT_BUSY`):
  0. Leave an open CLI first (§4.1: plain `exit` reboots; it only counts as a confirmed
     reboot if the FC printed `Rebooting` or acknowledged the fallback `MSP_REBOOT`), then
     refuse `PENDING_UNSAVED_CHANGES {reason}` if the RAM state (§4.1) is dirty. Checked
     before everything below; nothing is sent to the FC. Only a confirmed reboot clears it
     (`device_reboot`); a plain reconnect does not.
  1. Refuse `BOARD_ARMED` if armed. Betaflight itself ignores the request while armed but still
     ACKs it (msp.c 3238 / 3285 / 3314: `if (!ARMING_FLAG(ARMED)) accStartCalibration();`).
  2. Refuse `ACC_NOT_AVAILABLE {present, enabled}` if the ACC bit is clear or `acc_hardware =
     NONE` (also if the FC rejects the command, i.e. no `USE_ACC`).
  3. **Moving check**: 5 × `MSP_RAW_IMU` (gyro in deg/s) 40 ms apart; any axis > 8 °/s →
     `BOARD_MOVING {during: false}`, nothing sent. This catches handling/rotation, **not** a
     board that is still but not level, or slow sliding; the UI must still ask for a flat, level
     surface. (Accelerometer data is not used: it reads zero during calibration and its scale
     differs per sensor.)
  4. Send `MSP_ACC_CALIBRATION` (205). The FC averages 400 acc samples
     (`CALIBRATING_ACC_CYCLES`, acceleration_init.c 97 / 100 / 78) at the acc task rate
     (≈1 kHz), so ≈0.4–0.5 s.
  5. **Completion**: poll `MSP_STATUS_EX` every 50 ms; `ARMING_DISABLED_CALIBRATING` (bit 12) is
     set while any sensor calibrates (core.c `updateArmingStatus`, 4.5.5:335-337). Seen set then
     clear → `completion: CALIBRATING_FLAG`. If the flag cannot be used (no STATUS_EX, already
     set by another sensor such as a baro, or never caught) → wait 2 s like Configurator
     (`ACC_CALIBRATION_TIMEOUT_MS`) → `FIXED_WAIT`. Still set after 4 s →
     `ACC_CALIBRATION_FAILED`. Polls that time out while the FC writes its EEPROM are ignored.
     Gyro is sampled during the wait; > 8 °/s → `BOARD_MOVING {during: true}` (calibrate again).
  6. Re-read `MSP_BOARD_INFO`; "acc needs calibration" still set → `ACC_CALIBRATION_FAILED`.
  - **Auto-save**: the FC saves by itself. The last calibration cycle sets the trims and calls
    `saveConfigAndNotify()` (acceleration_init.c 4.4.3:445-455, 4.5.5:461-471,
    2025.12.5:428-438) = `writeEEPROM(); readEEPROM(); beeperConfirmationBeeps(1);` (config.c
    786 / 826 / 738). Side effect: **any other unsaved change in RAM is saved too**, which is
    why step 0 refuses while the RAM state is dirty (e.g. after `exit noreboot` + a failed
    `MSP_REBOOT` in config's cleanup).
  - `MSP_MAG_CALIBRATION` is not offered: it needs the craft rotated in all directions for 30 s
    and the algorithm/completion differs between 4.4 and 4.5+ (compass.c).
- **Attitude** — `MSP_ATTITUDE` (108): roll and pitch int16 decidegrees, yaw int16 whole degrees
  0..359 (msp.c 1332 / 1324 / 1323); sent as `f32` degrees with Betaflight's signs unchanged
  (Configurator's 3D model rotates by (−pitch, −yaw, −roll)). Signs: **roll + = right side
  down, pitch + = nose DOWN, yaw = heading, + = nose right** (imu.c 311-313; osd_elements.c
  2025.12.5:801 / 4.5.5:765 `pitch > 0 //nose down`; pid.c angle mode uses stick-sign error
  `angleTarget - attitude`, so pitch stick forward = nose down = positive). `device_attitude_stream_start`
  lends the port to a worker exactly like the RC stream: default 10 Hz, clamped 1–30;
  idempotent while running; motor test, RC stream and attitude stream are mutually exclusive
  (`PORT_BUSY {heldBy: "APP"}`); it ends with `device://attitude-stopped` on
  `device_attitude_stream_stop` / `device_disconnect` / app close (`REQUESTED`), port loss
  (`DISCONNECTED`) or 5 unanswered polls / another error (`ERROR`).
- **Board alignment is not live**: `MSP_SET_BOARD_ALIGNMENT_CONFIG` (39) only writes the
  parameter group (msp.c 3640-3644 / 3689-3693 / 3728-3732). The rotation matrix is built by
  `initBoardAlignment()` (boardalignment.c 51 / 66 / 64), which is called only from
  `fc/init.c` at boot (707 / 676 / 661), not from `activateConfig()`. So `MSP_ATTITUDE` reflects a
  new alignment only after save + reboot; a live "unsaved preview" must be done in the UI by
  rotating the displayed attitude.

## 5. Firmware dialects (config)

| Concern | 4.4 / 4.5 | 2025.12 | 2026.6 |
|---|---|---|---|
| `serial` port identifier | UART1=0 … (n-1); VCP=20; SOFTSERIAL1=30; LPUART1=40 | UART1=51 … (50+n); VCP=20; SOFTSERIAL1=30; LPUART1=40 | as 2025.12, plus UART11..15 = 61..65 |
| `save noreboot` | 4.4: no; 4.5: from 4.5.4 (backport PR #15151) | available | available |
| ESC idle setting | `dshot_idle_value` | `motor_idle` (PR #13936) | `motor_idle` |
| `feature -RX_MSP` | accepted | no CLI name (omit) | accepted again (`featureNames` restores `RX_MSP`, adds `RX_UDP`) |
| `expresslrs_rate_index` / `_switch_mode` | exist (never written) | removed (PR #12936) | removed |
| Function bitmask | same (see `config/serial_functions.rs`) | same | adds `LIDAR_NL` (1<<19), `OSD_CUSTOM_TEXT` (1<<20); never managed, always kept |
| Battery settings (`vbat_*`, `ibata_*`, …) | global | global | per battery profile (`PG_BATTERY_PROFILES`, 3 profiles); `dump`/`diff` print `battery_profile N` sections |
| Modes | — | ALTHOLD=3, POSHOLD=11 | adds AUTOPILOT=56, WP CAPTURE=57 (unmanaged, kept) |
| `MSP_BOARD_INFO` MCU type | MCU id | MCU id | 254 (`MCU_TYPE_ID_PROVIDED_BY_NAME`); the name is in `MSP2_MCU_INFO` (0x300C) |
| Beepers / OSD | 24 / 77 (4.5: 80) | 23 / 82 | 23 (unchanged) / 94 (adds `pos_hold_ready`, `battery_profile_name`, `custom_serial_text`, `wp_*` ×8, `nav_map`) |

Always take identifiers from `config_read_ports`; the table is only for tests.
Supported lines: **4.4.x, 4.5.x, 2025.12.x, 2026.6.x**. Everything else (≤ 4.3, 2026.7 and newer)
fails with `UNSUPPORTED_FIRMWARE` before any write.

**Single source of truth** (`config::firmware`, used by Device and the error text):

```rust
pub const SUPPORTED_LINES: &[(u16, u16)] = &[(4, 4), (4, 5), (2025, 12), (2026, 6)];
pub fn is_supported(version: &str) -> bool;      // major.minor in SUPPORTED_LINES; false if unparseable
pub fn supported_lines_label() -> String;        // "4.4.x, 4.5.x, 2025.12.x and 2026.6.x"
impl Dialect { pub fn line(self) -> (u16, u16); pub fn has_battery_profiles(self) -> bool; /* … */ }
```

The `UNSUPPORTED_FIRMWARE` message is built from `supported_lines_label()`; adding a line means
adding a `Dialect` variant and its `SUPPORTED_LINES` entry, nothing else in Device.

2026.6 notes:

- The generator writes no battery setting, so profiles are never switched. A backup's
  `battery_profile N` sections and a SITL/cloud build's `waypoint` lines are replayed verbatim by
  rollback (`guard::REPLAY_VERBS`). A `set vbat_*` written by hand only affects the profile that
  is currently selected (`cli.c` `cliBatteryProfile`, RAM only until `save`).
- `MSP_FC_VERSION` omits the version string on VTX (DisplayPort) ports; Config reads the
  version from the CLI `version` line, so it is unaffected.

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
  Both refuse if the backup's firmware line (4.4 / 4.5 / 2025.12 / 2026.6) differs from the FC's, or if
  the backup's `boardUid` and the FC's UID (`FcLink::uid()`) are both known and differ (`CONFIG`, nothing sent).
  The backup is always resolved by `id` inside the backups folder; the `path` field sent by the UI
  is never opened.
- **Backups** are stored at `<app_data_dir>/backups/<boardUid>/<UTC YYYYMMDDTHHMMSSZ>.txt` (on Windows
  `%APPDATA%\<identifier>\backups\…`) and are kept forever. `config_backups_dir` returns the root
  for the UI's "open backups folder" action. `<boardUid>` is `FcInfo.uid`, i.e. the 24-char
  `mcu_id` hex (e.g. `backups/0033002b3138510b32383733/`), so the folder matches what
  `mcu_id` prints in the CLI. Config takes it from `FcLink::uid()` (cached by Device at connect, no
  I/O; config never runs the `mcu_id` command); `unknown` if that is `None` (MSP_UID failed).
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
2. `flasher_fetch_firmware` (cache hit or Build API), with `options = FcInfo.buildOptions` and
   `requiredOptions = flasher_required_options(model)` (see "Build options" below).
3. `config_backup` (if FC runs Betaflight).
4. `device_reboot_to_bootloader` → poll `flasher_list_dfu_devices`; if `driverOk=false` or a
   flasher call fails with `DRIVER_MISSING {vid, pid}` → `system_install_dfu_driver` (UAC
   prompt, packaging) → retry.
5. `flasher_flash` (events). If it fails with `READ_PROTECTED` → confirmation screen ("this
   erases the chip") → `flasher_remove_read_protection {confirmErase: true}` → device
   re-enumerates → back to step 4.
6. After flashing → device re-enumerates as VCP → `device_connect` →
   `device_identify` → `config_read_ports`.

### Build options

Option values are the Build API's (`/api/options/{release}` → `value`, e.g. `USE_SERIALRX_CRSF`,
`USE_OSD_HD`); they match the firmware's `MSP_BUILD_INFO` names (`FcInfo.buildOptions`, §4.1).
The UI never names options itself.

- **Required options** (`flasher_required_options(requirements, release?)`, mapping owned by
  flasher in `flasher/options.rs`), from the `DroneModel` slice `{receiver: {kind, protocol}, osd: {videoSystem}}`:

  | Answer | Option |
  |---|---|
  | `receiver.kind = SERIAL`, protocol `CRSF` / `GHST` / `SBUS` / `FPORT` / `IBUS` / `SRXL2` / `SUMD` | `USE_SERIALRX_CRSF` / `_GHST` / `_SBUS` / `_FPORT` / `_IBUS` / `_SRXL2` / `_SUMD` |
  | `SERIAL` + `SPEK1024` / `SPEK2048` | `USE_SERIALRX_SPEKTRUM` |
  | `receiver.kind = PPM` | `USE_RX_PPM` |
  | `SPI` (`EXPRESSLRS`, `FRSKY_*`), `MSP`, `NONE` | – (SPI receivers come from the board's target config, not a Build API option; MSP RX needs none) |
  | `osd.videoSystem = HD` | `USE_OSD_HD` |
  | `osd.videoSystem = PAL` / `NTSC` / `AUTO` | `USE_OSD_SD` |
  | `osd` null / `videoSystem` null | – |

  The names are the same on 4.4.3, 4.5.x and 2025.12.x. With `release`, the result is also checked
  against that release's list.
- **Resolution in `flasher_fetch_firmware`:** fetch the release's option list (stored under the
  firmware cache, `api/options-<release>.json`, and reused when offline or the API fails). Base =
  `options` (board) if non-empty, else the release's `default` options. Keep base options the
  release offers; the rest go to `FirmwareImage.droppedOptions` (logged), e.g. 2025.12 → 4.5.2
  drops `USE_ALTITUDE_HOLD`, `USE_POSITION_HOLD`, `USE_SOFTSERIAL`, `USE_WING`, and 4.4.3 also
  `USE_RACE_PRO`. Required options are always added; one the release does not offer →
  `REQUIRED_OPTION_UNSUPPORTED`. No usable option list → `BUILD_OPTIONS_UNAVAILABLE`, never a
  stripped build. The firmware cache key uses the resolved list.

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
  lack it (one UAC prompt: the app relaunches its own exe with an elevated-install
  subcommand that `main.rs` dispatches before Tauri starts; the app UI never runs
  elevated). Driver files are
  extracted to a per-run SYSTEM/Administrators-only folder under `%ProgramData%`. Details, build and
  LGPL-3.0 obligations: `docs/PACKAGING.md`.
  device/flasher never install drivers; they report `DRIVER_MISSING` with the device's
  `{vid, pid}` (§2.3).
- CI (`.github/workflows/build.yml`): on push/PR → windows-latest: `npm ci`, `npm test`,
  `npm run build`, `cargo test`, `tauri build` (artifacts uploaded); ubuntu: `cargo check`.
- Rust toolchain: stable ≥ 1.90 (Tauri 2.12 requirement). Node 22.
