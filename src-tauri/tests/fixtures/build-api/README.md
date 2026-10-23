# Build API option lists

`options-<release>.json`: replies of `GET https://build.betaflight.com/api/options/<release>`
for 4.4.3, 4.5.2 and 2025.12.2, fetched on 2026-10-23 and re-indented (content unchanged).
They are lists of Betaflight build-option names, defines and MSP_BUILD_INFO ids (the defines come
from the GPL-3.0 Betaflight firmware). Used by `src/flasher/options.rs` and `service.rs` tests.

2026.6 (fetched 2026-10-23, re-indented, content unchanged):

- `options-2026.6.2.json`: `GET /api/options/2026.6.2` (identical to `/api/options/2026.6`). Same
  lists as 2025.12.5 plus `USE_FLIGHT_PLAN` (key 16427); `USE_RX_PPM`, `USE_SERIALRX_*`
  (incl. `USE_SERIALRX_SPEKTRUM`), `USE_OSD_HD` and `USE_OSD_SD` are unchanged.
- `target-SPEEDYBEEF405V4.json`: `GET /api/targets/SPEEDYBEEF405V4` (release list: 2026.12.0-alpha
  Unstable, 2026.6.2/2026.6.1 Stable, 2025.12.x, 4.5.x, ...).
- `build-2026.6.2-SPEEDYBEEF405V4.json`: `GET /api/builds/2026.6.2/SPEEDYBEEF405V4` (target
  detail: `cloudBuild`, `mcu` "STM32F405", `releaseType`, `configHash`, ...).
