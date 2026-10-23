# MVP task list (Phase 1)

Each item is meant to become one GitHub issue with the workstream label.
`scripts/create-mvp-issues.sh` creates them (labels + issues) with `gh`.

## device
- **D1 – Port enumeration + VID/PID classification**: implement `device_list_ports` with `serialport`; classify via `usb_ids.rs`; unit tests for the table.
- **D2 – Serial connection manager**: `device_connect` / `device_disconnect`, single active connection in `tauri::State`, reconnect after FC reboot (VCP re-enumeration).
- **D3 – MSP v1/v2 client**: framing, checksum/CRC8, request/response matching, timeouts; implement `Connection::msp_request`; tests with byte fixtures.
- **D4 – FC identification**: `device_identify` via MSP_API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, UID, BUILD_INFO → `FcInfo`.
- **D5 – CLI mode**: `enter_cli`/`cli_exec`/`exit_cli` (prompt detection, `###ERROR###` capture), `device_cli_exec` command.
- **D6 – Reboot to bootloader**: `device_reboot_to_bootloader` (MSP_SET_REBOOT bootloader type, fallback CLI `bl`).

## flasher
- **F1 – Build API client**: targets, releases, options, build request/poll/download (`build.betaflight.com`); offline target list cache.
- **F2 – Firmware cache**: content-addressed cache in `%LOCALAPPDATA%`, SHA-256, `flasher_list_cache`, `offlineOnly` behaviour.
- **F3 – DFU enumeration**: `flasher_list_dfu_devices` (rusb), parse alt-setting descriptor → MCU family + flash layout, `driverOk` detection.
- **F4 – DFU flashing STM32 F4/F7/H7 + AT32F435**: Intel HEX parsing, erase (full/sector), write, verify, leave DFU; `flasher://progress` events.
- **F5 – Flash orchestration & recovery**: identify → fetch → backup hook → bootloader → flash → reconnect; recover if FC stuck in DFU.

## config
- **C1 – Mock Connection + CLI transcript fixtures**: `MockConnection` implementing `device::Connection` that replays recorded CLI/MSP transcripts for 4.4, 4.5 and 2025.12 (`src-tauri/tests/fixtures`).
- **C2 – `config_read_ports`**: parse `serial` output (+ names via `resource show`/MSP) → `FcSerialPort[]` with version-specific identifiers.
- **C3 – `generate()` dialects**: DroneModel → CLI for 4.4/4.5/2025.12 (serial functions, receiver, VTX, GPS, ESC, aux); conflict warnings; golden tests.
- **C4 – Backup**: `config_backup` (`diff all` to file with metadata).
- **C5 – Apply**: batch apply, collect errors, save, return `ApplyResult`.
- **C6 – Verify + rollback**: read back and compare (`config_verify`); `config_rollback` from backup.

## ui
- **U1 – App shell**: step navigation driven by `steps.json`, validation gating Next, he/en switch with RTL, error display by `AppError.code`.
- **U2 – Connect step**: port list, auto-detect, DFU/no-device states, driver-help screen.
- **U3 – Firmware step**: target search filtered by MCU, release picker, progress UI on `flasher://progress`.
- **U4 – Components + Receiver/VTX/GPS steps**: only free ports offered, conflict hints.
- **U5 – ESC + Switches steps**: protocol/bidir/poles/props-out; ARM switch detection (live RC channels via MSP, mocked until device lands).
- **U6 – Review/Apply/Done**: summary, CLI preview, apply/verify progress, rollback button.
- **U7 – Hebrew/English copy**: full strings, RTL layout review.

## packaging
- **P1 – CI green on windows-latest**: tests, build, NSIS+MSI artifacts; Linux check job.
- **P2 – WinUSB driver install**: libwdi-based elevated helper as Tauri sidecar; `system_driver_status`, `system_install_dfu_driver`.
- **P3 – Installer polish**: Hebrew/English NSIS, icons, app metadata, WebView2 bootstrapper.
- **P4 – Release pipeline**: tag → GitHub Release with installers; code-signing plan.
