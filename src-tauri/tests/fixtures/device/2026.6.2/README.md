# Betaflight 2026.6.2 SITL capture (device)

`msp.txt` was captured from a Betaflight 2026.6.2 SITL (tag 2026.6.2, commit e0b7bb01b,
built with `tests/sitl/build-sitl.sh 2026.6.2`) by `tests/sitl/capture.py`. Lines are
`NAME code payload-hex`. The device tests in `src/device/msp.rs` replay it.

Notable 2026.6 content:

- `FC_VERSION` is `[26, 6, 2]` followed by the text version "2026.6.2".
- In `BOARD_INFO`, mcu type id 254 = `MCU_TYPE_ID_PROVIDED_BY_NAME`. The name comes from
  `MSP2_MCU_INFO` (0x300C: u8 id, then a pstring). On SITL that is "SIMULATOR". `capture.py`
  does not request MSP2_MCU_INFO, so the replay test answers it.
- `BUILD_INFO` lists 16427 (`USE_FLIGHT_PLAN`) twice, because upstream `msp_build_info.h`
  defines it twice.

The CLI captures (`dump all` etc.) are missing because `capture.py` rejects the 2026.6
`dump all` as not well-formed, so `msp.txt` is all there is. To check the CLI read path,
run the ignored live test `device::sitl_smoke_tests`.
