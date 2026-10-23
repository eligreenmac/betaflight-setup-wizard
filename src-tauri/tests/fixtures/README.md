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
| 2026.6.2 | `2026.6.2` | `e0b7bb01b17b21351057e9ead2d1ab39dd44fa16` |

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
- **`save` after a failed batch still saves** (all four versions). The engine therefore
  sends `exit`, never `save`, once any command failed. MockFc models this.
- 2025.12 `MSP_FC_VERSION` is `yy month patch` + a length-prefixed version string
  (`19 0c 05 09 "2025.12.5"`); decoders must use the string.
- `MSP_UID` is three LE u32 words; rendered `%08x` × 3 it equals the CLI `mcu_id`.
- `dshot_idle_value` (4.x) only exists with DSHOT; 2025.12+ has `motor_idle` always.
- 2026.6: MSP API 1.48 (`00 01 30`); `MSP_BOARD_INFO` MCU type is `fe` (254,
  `MCU_TYPE_ID_PROVIDED_BY_NAME`, build/build_config.h), the name comes from
  `MSP2_MCU_INFO`. `dump all` / `diff all` have `battery_profile 0..2` sections and a
  `# restore original battery_profile selection` block; the SITL build (USE_FLIGHT_PLAN,
  common_post.h) also prints `waypoint` lines and `ap_*` settings. `feature` lists `RX_MSP`
  again and adds `RX_UDP`. `capture.py` accepts the new sections.
  `resource` is not compiled into SITL (it is on every F4/F7 build).

## 2. Board definitions: `boards/`

| File | Source | Revision |
|---|---|---|
| `boards/unified-targets/SPBE-SPEEDYBEEF405V4.config`, `HAMO-CRAZYBEEF4SX1280.config` | https://github.com/betaflight/unified-targets `configs/default/` | `2ac098da` (2026-07-15) |
| `boards/config/SPEEDYBEEF405V4/config.h`, `CRAZYBEEF4SX1280/config.h` | https://github.com/betaflight/config `configs/` | `7b1f01a25d8c` (2026-07-18, branches `4.5-config` = `2025.12-config`; identical to the 2025.12.5 submodule pin `92c695af` and to the 2026.6.2 `src/config/configs/{SPBE/SPEEDYBEEF405V4,HAMO/CRAZYBEEF4SX1280}/config.h` for these boards) |

Verbatim, GPL-3.0. 4.4 builds use the unified-target `.config`; 4.5, 2025.12 and 2026.6
build the board from `config.h`.

## 3. Composed board dumps: `4.4/`, `4.5/`, `2025.12/`, `2026.6/` `*.dump.txt`

Written by `tests/sitl/compose.py`, three layers:

1. the SITL `dump all` of that version (all settings, features, defaults);
2. the board file: banner MCU / identifier, `board_name`, `manufacturer_id`, resources,
   UARTs and their default functions (`MSP_UART` → MSP, `SERIALRX_UART` → RX_SERIAL,
   `ESC_SENSOR_UART` → ESC_SENSOR, or the `.config` `serial` lines), default RX feature,
   softserial when the feature is on;
3. an overlay of defaults that SITL cannot compile (DSHOT, GPS, OSD, RX_SPI/ELRS),
   each value with the firmware source file it comes from (see `OVERLAY` in compose.py),
   including `gps_sbas_mode` / `gps_ublox_use_galileo` (pg/gps.c).

The banner carries a `# COMPOSED FIXTURE (not a hardware recording)` line.

Batch 2 (beepers, OSD) adds to the overlay, each from the firmware source:

- `beeper` / `beacon` sections (`io/beeper.c` `beeperTable`, all beepers on, both DShot
  beacon conditions off). 2025.12 prints `CRASHFLIP` (4.x: `CRASH_FLIP`) and has no
  `RC_SMOOTHING_INIT_FAIL`: 24 / 24 / 23 / 23 lines (2026.6 = 2025.12).
- every `osd_<element>_pos` the board's build has (`cli/settings.c`, defaults from
  `osd/osd.c` `pgResetFn_osdElementConfig`). Defaults are hidden at the centre of the
  default canvas: 4.4 SD (15,7), e.g. `osd_vbat_pos = 234`; 4.5 / 2025.12 HD (26,10),
  `osd_vbat_pos = 341`. `osd_warnings_pos` is the one visible default (all profile bits)
  at (centre-6, centre+3): 14665 / 14772. `gps_lap_*` need `USE_GPS_LAP_TIMER` (cloud-build option, not included);
  `lidar_dist` needs `USE_RANGEFINDER`, which 2025.12 enables only with >= 1024 KB flash
  (F405 yes, F411 no).
- 2026.6 adds `osd_custom_serial_text_pos` (every OSD build), `osd_battery_profile_name_pos`
  (`USE_PROFILE_NAMES`) and `osd_pos_hold_ready_pos` (`USE_POSITION_HOLD`, >= 1024 KB flash:
  F405 yes, F411 no). `osd_wp_*` / `osd_nav_map_pos` need `USE_FLIGHT_PLAN` (SITL / cloud
  builds only), so the board dumps drop them, together with the SITL-only `waypoint` lines
  and `ap_*` settings (`SITL_ONLY_*` in compose.py). Other defaults are the 2025.12 ones
  (osd.c 438-472). DShot default is DSHOT600 on F405/F411 (pg/motor.c:66); the GPS provider
  default is UBLOX on boards (VIRTUAL only on SITL, pg/gps.c).
- `osd_displayport_device`: AUTO on 4.4, **MSP on 4.5 / 2025.12 / 2026.6** (`USE_OSD_HD` is in
  `common_pre.h`), the earlier fixtures had AUTO there. `osd_canvas_width/height` 53/20
  exist on 4.4 / 4.5 only (2025.12+: `#ifdef OSD_CANVAS_SIZE_DEBUG`).
  `vcd_video_system = AUTO`, `displayport_msp_col/row_adjust = 0`.

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
- The composed dumps keep some SITL-build settings a board build would not have
  (e.g. `pos_hold_*` on the F411, `gps_*` everywhere, `mag_hardware`), as for the older lines.
- MockFc prints the 2026.6 `battery_profile 0..2` section headers and accepts
  `battery_profile N`, but keeps one flat settings map (no per-profile values) and does not
  model `waypoint`.
- MockFc `beeper` / `beacon` answer like `cli.c` (`Enabled X` / `Disabled X`, prefix match
  in table order, `INVALID NAME`); `beeper` without arguments and `beeper list` are not
  modelled.

## 5. Opt-in end-to-end test against SITL

```
tests/sitl/build-sitl.sh 4.5.5 /tmp/bfsw-sitl     # prints the .elf path
BFSW_SITL_BIN=<elf> cargo test sitl_end_to_end -- --test-threads=1
```

`src/config/tests_sitl_e2e.rs`: MSP identification, apply (backup, batch, `save`, process
restart), a rejected command (nothing saved), rollback to the first backup, and the
model-extension lines (`vtxtable`, `vtx_band/channel/power`, `craft_name`,
`motor_output_reordering`, 2025.12+ `motor_idle`) read back from `dump all`. GPS and DShot
are not in SITL, so `gps_sbas_mode`, `gps_ublox_use_galileo` and `dshot_idle_value` are
checked by MockFc and goldens only. Skipped when `BFSW_SITL_BIN` is unset. Passes locally
on 4.4.3, 4.5.5, 2025.12.5 and 2026.6.2.

Step 9 (batch 2) runs plan → apply → verify for `batch2-freestyle-hd` (airmode SWITCH,
removeModes with an old PREARM range, a kept unmanaged range, sensors, board alignment).
SITL has no `beeper` / `beacon` commands and no OSD settings, so the plan must warn for the
beepers and fail with CONFIG for the OSD; serial / DShot / GPS lines are left out of the
applied script. Beeper and OSD lines are covered by MockFc and the goldens.

SITL quirk: an 8-character VTX band name reads back with its last character replaced by
a space (`BOSCAM_A` → `BOSCAM_`) on all four versions, although
`VTX_TABLE_BAND_NAME_LENGTH` is 8 and real dumps show `BOSCAM_A` / `RACEBAND`. The e2e
test uses 7-character names; the generator keeps the 8-character limit.

## 6. Public references (not included, no licence grant)

Used only to cross-check the composed fixtures:

- SpeedyBee F405 V4, 4.5.1 `diff all` and 4.5.2 `diff all defaults`, IntoFPV forum
  (https://intofpv.com/t-vtx-fail--28232, attachments 13562 / 13563). Agrees with the
  composed fixture on the banner format `# Betaflight / STM32F405 (S405) 4.5.x …`,
  `board_name SPEEDYBEEF405V4` / `manufacturer_id SPBE`, UART1 default function `0`, and
  UART5 being RX-only (`SERIAL_TX 5 NONE`). Real dumps also have a `# config rev:` line.
- CrazyBee F4 SX1280, 4.3.0 `diff all` in betaflight/betaflight#11448 (SPI ExpressLRS,
  `dshot_idle_value`, `vtxtable` block) and the related issue betaflight/betaflight#13581.
