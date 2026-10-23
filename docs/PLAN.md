# Betaflight Setup Wizard — Product Plan

**Goal:** a Windows desktop app that lets *any* pilot, with *any* Betaflight-supported
flight controller (FC), go from "board in hand" to "configured quad" without visiting a
website: auto-detect the FC, flash Betaflight, ask plain questions about what is wired
where, and apply the right settings automatically. Hebrew (RTL) and English UI.

License: **GPL-3.0-or-later** (we reuse logic/knowledge from the GPL-3.0
Betaflight App / Configurator).

## 1. Background (research summary)

| Finding | Source |
|---|---|
| The official Betaflight App is a Vue/Vite PWA (app.betaflight.com) and also ships desktop installers built with **Tauri 2** (e.g. `Betaflight-2026.6.2-x64-setup.exe`). GPL-3.0. | https://github.com/betaflight/betaflight-configurator , https://github.com/betaflight/betaflight-configurator/releases |
| Firmware 4.4+ is built on demand by the cloud **Build API** `https://build.betaflight.com` (`/api/targets`, `/api/builds/<release>/<target>`, `POST /api/builds`, `/api/builds/<key>/status`, `/api/builds/<key>/json`). ~650 targets with `manufacturer` + `mcu`. | `src/js/BuildApi.ts` in the configurator; https://build.betaflight.com/api/targets |
| FC auto-detect: connect via MSP, read `board_name`, match to Build API targets. | `src/js/utils/AutoDetect.ts` |
| USB VID/PIDs for FC VCPs and USB-UART bridges (STM32, AT32, APM32, GD32, RP2040, CP210x, CH340, FTDI); DFU = `0483:DF11` (STM32), `2E3C:DF11` (AT32). | `src/js/protocols/devices.ts`, `usbdfu.ts` |
| Flashing: STM32/AT32 ROM USB DFU, serial bootloader, ESP32; local `.hex/.bin/.uf2`. | `src/js/protocols/{usbdfu,webstm32,esp32}.ts` |
| Config is applied via CLI (`#`, `batch start`, …, `batch end`, `save`) or MSP. | https://betaflight.com/docs/wiki/guides/current/Cli |
| Serial function bitmask is stable (MSP=1, GPS=2, RX_SERIAL=64, ESC_SENSOR=1024, SMARTAUDIO=2048, TRAMP=8192, VTX_MSP=131072). **Port identifiers changed**: 4.4/4.5 use UART1=0, 2025.12+ use UART1=51 (VCP=20, SOFTSERIAL1=30, LPUART1=40 in both). | `src/main/io/serial.h` (4.5-maintenance vs 2025.12-maintenance/master) |
| Related projects (none is an end-to-end Windows wizard): SebGalina/betaflight-claude-skill (AI CLI generator), formicking/fvp-cli-configurator (abandoned), betaflight/firmware-presets (preset content). | GitHub |

## 2. Architecture (summary — details in ARCHITECTURE.md)

**Tauri 2 desktop app** — Rust core + Vue 3 / Vite / TypeScript UI.

- **Rust core** (`src-tauri/src`): `device` (USB/serial/MSP/CLI), `flasher` (Build API +
  cache + DFU), `config` (DroneModel → CLI, backup/apply/verify/rollback), `system`
  (Windows drivers, environment).
- **UI** (`src/`): data-driven wizard (`src/wizard/steps.json` + component registry),
  typed `invoke()` wrapper (`src/api`) with a mock backend, vue-i18n he/en with RTL.
- **Shared contract**: `DroneModel` JSON schema (`schemas/drone-model.schema.json`) +
  Tauri command names (ARCHITECTURE.md §3).
- **Packaging**: NSIS + MSI via Tauri bundler, WinUSB driver install via libwdi, GitHub
  Actions on `windows-latest`.

Why not fork the configurator: it is large and changes daily; we need ~5% of it
(MSP, DFU, Build API client). We port those pieces into Rust under GPL-3.0.

## 3. Universal-product principles

1. **Never assume hardware.** Board identity comes from MSP (`board_name`, MCU) or, in
   DFU mode, from the DFU descriptor (MCU family) + a searchable target list filtered by MCU.
2. **Never hard-code ports.** Available UARTs and their firmware identifiers are read
   from the FC after flashing (`config_read_ports`); the wizard only offers free ports.
3. **Version-aware CLI dialects.** `config::generate` picks a dialect from the firmware
   version (4.4, 4.5, 2025.12, …). Unknown versions → refuse or warn, never guess silently.
4. **Safety first.** Always back up (`diff all`) before changes; verify after; one-click
   rollback; "props off" gates for any motor test.
5. **Offline-first.** Firmware is cached after first download; optional offline packs.

## 4. Wizard flow

1. **Welcome** — language, props-off warning.
2. **Connect** — list ports (classified by VID/PID). Outcomes:
   Betaflight detected (board, version) / DFU device (MCU family) / nothing → driver help.
3. **Firmware** — keep current, or update/flash: choose release; build options derived
   from later answers (e.g. GPS); fetch (cache or Build API) → backup → reboot to DFU →
   flash → reconnect → re-identify.
4. **What is connected?** — checkboxes: receiver, VTX/digital air unit, GPS, ESC
   telemetry wire, LED strip, buzzer.
5. **Receiver** — kind (serial/SPI/…), protocol (ELRS/CRSF, SBUS, IBUS, GHST, FPort, …),
   UART pad (from free ports), telemetry.
6. **VTX** — analog (SmartAudio/Tramp + region) or digital (DJI/Walksnail/HDZero → MSP
   DisplayPort), UART pad.
7. **GPS** — UART pad, provider.
8. **ESC & motors** — protocol, bidirectional DShot, motor poles, props-out; (phase 2:
   per-motor direction test with props-off gate).
9. **Switches** — move a switch → detect AUX channel → assign ARM/ANGLE/BEEPER….
10. **Review** — human summary + generated CLI preview + warnings (port conflicts).
11. **Apply** — backup → batch apply → save → reconnect → verify.
12. **Done** — report + backup file location; rollback button.

Example generated script (firmware 4.5, ELRS on UART2, DJI on UART1, GPS on UART4):

```
batch start
serial 0 131072 115200 57600 0 115200
serial 1 64 115200 57600 0 115200
serial 3 2 115200 57600 0 115200
set serialrx_provider = CRSF
feature RX_SERIAL
feature TELEMETRY
feature GPS
set gps_provider = UBLOX
set osd_displayport_device = MSP
set motor_pwm_protocol = DSHOT600
set dshot_bidir = ON
set motor_poles = 14
set yaw_motors_reversed = ON
aux 0 0 0 1800 2100 0 0
batch end
save
```
(On 2025.12+ the same ports are `serial 51 …`, `serial 52 …`, `serial 54 …`.)

## 5. Offline firmware strategy

- **Default:** Build API on first use → content-addressed cache (`%LOCALAPPDATA%`), keyed
  by target + release + sorted options; SHA-256 verified.
- **Offline packs (phase 2):** downloadable zip of "standard" builds per release, split by
  MCU family / popular boards. Full set estimated at several hundred MB–~1 GB (to measure).
- **Local build (phase 4, advanced):** bundled arm-none-eabi-gcc + betaflight source +
  betaflight/config → `make CONFIG=<board>`.
- **Open item:** confirm Build API terms of use for third-party clients.

## 6. MVP scope

In:
- STM32 + AT32 FCs, Betaflight 4.4 / 4.5 / 2025.12.
- Auto-detect (MSP + VID/PID + DFU), Build API fetch + cache, USB DFU flash with
  bootloader entry and recovery.
- Wizard: receiver (serial + SPI ELRS), VTX (SmartAudio/Tramp/MSP), GPS, ESC protocol +
  bidir + poles + props-out, ARM switch.
- Backup → apply → verify → rollback.
- he/en UI with RTL; Windows NSIS/MSI installer; WinUSB driver install.

Out (later phases): offline packs, serial bootloader, ESP32/UF2, motor direction test,
full mode mapping, presets/tuning, OSD layout, LED strip, local firmware build.

## 7. Phases

| Phase | Content |
|---|---|
| 0 — Foundation (this commit) | Skeleton, contracts, CI, issues per workstream |
| 1 — MVP | Scope in §6 |
| 2 | Offline packs, serial bootloader, motor direction test, switch mapping for all modes, VTX tables per region |
| 3 | Official presets (rates/filters by frame size), GPS Rescue, basic OSD, saved profiles, auto-update, code signing |
| 4 | ESP32/UF2/RP2350 targets, local build, diagnostics/telemetry, more languages |

## 8. Workstreams

| Workstream | Branch | Owns | Label |
|---|---|---|---|
| Device & Protocol | `feat/device` | `src-tauri/src/device` | `device` |
| Firmware Flashing + Firmware Service | `feat/flasher` | `src-tauri/src/flasher` | `flasher` |
| Config Engine | `feat/config` | `src-tauri/src/config` | `config` |
| Wizard UX/UI + i18n | `feat/ui` | `src/` | `ui` |
| Windows Packaging, Drivers, CI | `feat/packaging` | `src-tauri/src/system`, `tauri.conf.json`, `.github/` | `packaging` |
| QA & hardware lab, PM/licensing | (cross-cutting) | `tests/`, docs | — |
