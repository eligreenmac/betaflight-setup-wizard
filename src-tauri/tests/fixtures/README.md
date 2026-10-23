# Config test fixtures: sources, licences, gaps

C1 / #12. None of these files is a recording from real flight-controller hardware.
Every file is either a **verbatim copy of public GPL-3.0 Betaflight material**, a
**capture from Betaflight SITL** built from an official release tag, or **composed** from
those two by a script in this repo. User-posted dumps from forums / issues have no licence
grant, so they are **not** included; they are listed at the end only as references used
to cross-check the composed fixtures.

All Betaflight material is GPL-3.0-only, the same licence as this app.

## 1. SITL captures: `sitl/<version>/`

| Version | Tag | Commit (betaflight/betaflight) |
|---|---|---|
| 4.4.3 | `4.4.3` | `738127e7e11da587f63b9531ec732a37bdb436b0` |
| 4.5.5 | `4.5.5` | `4adbd3ef7cb546947600e5f747bd5453c9573063` |
| 2025.12.5 | `2025.12.5` | `7348054f268f0058574719c134e9f149565bb8ea` |

Source: https://github.com/betaflight/betaflight (GPL-3.0). Built with
`tests/sitl/build-sitl.sh <tag>`, which builds `TARGET=SITL` and appends the wizard
profile defines (`USE_SERIALRX_CRSF/SBUS/GHST/...`, `USE_VTX_*`, `USE_MSP_DISPLAYPORT`, …)
to the SITL `target.h`. GPS, OSD, DSHOT and RX_SPI do not compile for SITL. Captured with
`tests/sitl/capture.py <elf> <outdir>` over TCP 5761 (UART1), on 2026-10-23; outputs are
LF-normalised. Large outputs over TCP are sometimes garbled, so `dump all` / `diff all`
are retried until every line is well formed.

| File | Content |
|---|---|
| `msp.txt` | MSP v1 replies `NAME code hex` for API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, BUILD_INFO, UID |
| `version.txt`, `serial.txt`, `feature.txt` | the CLI commands' output |
| `dump_all.txt`, `diff_all.txt` | defaults of the SITL build |
| `transcript.txt` | real responses to valid/invalid `get`, `set`, `feature`, `serial`, `aux`, `batch`, unknown commands |
| `save.txt` | `save`, `save` after a failed batch, and `exit`, each across a process restart |

Findings that tests now pin (`src/config/tests_sitl.rs`):

- The CLI echoes every line; errors are `###ERROR IN <cmd>: <reason>###`, e.g.
  `###ERROR IN set: INVALID NAME: foo = 1###`,
  `###ERROR IN input: UNKNOWN COMMAND, TRY 'HELP'###`. A compiled-out feature answers
  `unavailable` (4.x) / `Unavailable GPS` (2025.12) **without** `###ERROR`.
- `batch end` after an error prints
  `###ERROR IN batch: ERRORS WERE DETECTED - PLEASE REVIEW BEFORE CONTINUING###`.
- **`save` after a failed batch still saves** (all three versions). The engine therefore
  sends `exit`, never `save`, once any command failed. MockFc models this.
- 2025.12 `MSP_FC_VERSION` is `yy month patch` + a length-prefixed version string
  (`19 0c 05 09 "2025.12.5"`); decoders must use the string.
- `MSP_UID` is three LE u32 words; rendered `%08x` × 3 it equals the CLI `mcu_id`.
- `dshot_idle_value` (4.x) only exists with DSHOT; 2025.12 has `motor_idle` always.
  `resource` is not compiled into SITL (it is on every F4/F7 build).

## 2. Board definitions: `boards/`

| File | Source | Revision |
|---|---|---|
| `boards/unified-targets/SPBE-SPEEDYBEEF405V4.config`, `HAMO-CRAZYBEEF4SX1280.config` | https://github.com/betaflight/unified-targets `configs/default/` | `2ac098da` (2026-07-15) |
| `boards/config/SPEEDYBEEF405V4/config.h`, `CRAZYBEEF4SX1280/config.h` | https://github.com/betaflight/config `configs/` | `7b1f01a25d8c` (2026-07-18, branches `4.5-config` = `2025.12-config`; identical to the 2025.12.5 submodule pin `92c695af` for these boards) |

Verbatim, GPL-3.0. 4.4 builds use the unified-target `.config`; 4.5 and 2025.12 build
the board from `config.h`.

## 3. Composed board dumps: `4.4/`, `4.5/`, `2025.12/` `*.dump.txt`

Written by `tests/sitl/compose.py`, three layers:

1. the SITL `dump all` of that version (all settings, features, defaults);
2. the board file: banner MCU / identifier, `board_name`, `manufacturer_id`, resources,
   UARTs and their default functions (`MSP_UART` → MSP, `SERIALRX_UART` → RX_SERIAL,
   `ESC_SENSOR_UART` → ESC_SENSOR, or the `.config` `serial` lines), default RX feature,
   softserial when the feature is on;
3. an overlay of defaults that SITL cannot compile (DSHOT, GPS, OSD, RX_SPI/ELRS),
   each value with the firmware source file it comes from (see `OVERLAY` in compose.py).

The banner carries a `# COMPOSED FIXTURE (not a hardware recording)` line.

Consequences now covered by tests: the SpeedyBee F405 V4 ships with UART2 = RX_SERIAL,
UART4 = MSP and UART5 = ESC_SENSOR, so a plan that puts GPS on UART4 must report the
existing MSP function (`board_default_msp_port_is_not_taken_silently`), and plans clear
the default ESC-sensor function on UART5 when the model has no ESC telemetry (goldens).

## 4. Gaps

- No hardware recordings: timing, USB-VCP framing, real reboot/re-enumeration and DFU are
  not covered. SITL has no VCP; its MSP port is UART1.
- DSHOT / GPS / OSD / ELRS values in the composed dumps come from source defaults, not
  from a running board.
- `resource` lists only the pins the board file assigns (a real `dump` also lists `NONE`
  entries), and there is no `dma`/`timer` section.
- MockFc enforces lookup values only for `serialrx_provider` (captured per version) and
  ON/OFF settings; other `set` values are accepted unchecked. `get` prints only the value
  line, not "Allowed values" / "Default value".
- MockFc BOARD_INFO / BUILD_INFO carry fixed hardware revision, capabilities, signature,
  gyro rate and an empty build-option list.
- Only two boards (F405 UART RX, F411 SPI ELRS).

## 5. Opt-in end-to-end test against SITL

```
tests/sitl/build-sitl.sh 4.5.5 /tmp/bfsw-sitl     # prints the .elf path
BFSW_SITL_BIN=<elf> cargo test sitl_end_to_end -- --test-threads=1
```

`src/config/tests_sitl_e2e.rs`: MSP identification, apply (backup, batch, `save`, process
restart), a rejected command (nothing saved), and rollback to the first backup. Skipped
when `BFSW_SITL_BIN` is unset. Passes locally on 4.4.3, 4.5.5 and 2025.12.5.

## 6. Public references (not included, no licence grant)

Used only to cross-check the composed fixtures:

- SpeedyBee F405 V4, 4.5.1 `diff all` and 4.5.2 `diff all defaults`, IntoFPV forum
  (https://intofpv.com/t-vtx-fail--28232, attachments 13562 / 13563). Agrees with the
  composed fixture on the banner format `# Betaflight / STM32F405 (S405) 4.5.x …`,
  `board_name SPEEDYBEEF405V4` / `manufacturer_id SPBE`, UART1 default function `0`, and
  UART5 being RX-only (`SERIAL_TX 5 NONE`). Real dumps also have a `# config rev:` line.
- CrazyBee F4 SX1280, 4.3.0 `diff all` in betaflight/betaflight#11448 (SPI ExpressLRS,
  `dshot_idle_value`, `vtxtable` block) and the related issue betaflight/betaflight#13581.
