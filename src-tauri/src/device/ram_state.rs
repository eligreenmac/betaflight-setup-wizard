// SPDX-License-Identifier: GPL-3.0-or-later
//! Which link operations may leave unsaved changes in the FC's RAM (ARCHITECTURE.md §4.1
//! "RAM state"). Used by the connection to keep its dirty-RAM flag; deliberately
//! conservative: anything not known to be read-only counts as a change.

use super::msp::codes;

/// CLI commands that only print, whatever their arguments (`dump all`, `diff hardware`,
/// `get gyro`, ...).
const CLI_READ_ONLY: &[&str] = &["diff", "dump", "get", "help", "status", "tasks", "version"];

/// CLI commands that only print when given no arguments (with arguments they set things).
const CLI_LIST_WITHOUT_ARGS: &[&str] = &[
    "adjrange",
    "aux",
    "beacon",
    "beeper",
    "board_name",
    "color",
    "dma",
    "feature",
    "led",
    "manufacturer_id",
    "map",
    "mixer",
    "mmix",
    "mode_color",
    "name",
    "profile",
    "rateprofile",
    "resource",
    "rxfail",
    "rxrange",
    "serial",
    "servo",
    "signature",
    "smix",
    "timer",
    "vtx",
    "vtxtable",
];

/// True if `line` cannot change any setting. Commands are case-insensitive in Betaflight.
/// `batch start`, `save`, `defaults`, `exit`, `bl` and everything unknown are not read-only.
pub fn cli_line_is_read_only(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return true;
    }
    let mut words = line.split_whitespace();
    let cmd = words.next().unwrap_or_default().to_ascii_lowercase();
    let has_args = words.next().is_some();
    CLI_READ_ONLY.contains(&cmd.as_str())
        || (!has_args && CLI_LIST_WITHOUT_ARGS.contains(&cmd.as_str()))
}

/// MSP requests answered by Betaflight's `mspProcessOutCommand` / `mspCommonProcessOutCommand`
/// (reads), from `msp_protocol.h` / `msp_protocol_v2_*.h`. Sorted for binary search.
const MSP_READS: &[u16] = &[
    1, 2, 3, 4, 5, 10, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 59, 61, 64, 70, 71,
    75, 77, 79, 80, 84, 86, 88, 90, 92, 94, 96, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110,
    111, 112, 116, 117, 118, 119, 120, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 135, 137,
    138, 139, 140, 145, 150, 160, 164, 167, 169, 170, 184, 189, 238, 240, 241, 247, 253, 254,
    0x1009, // MSP2_COMMON_SERIAL_CONFIG
    0x3001, // MSP2_MOTOR_OUTPUT_REORDERING
    0x3004, // MSP2_GET_VTX_DEVICE_STATUS
    0x3005, // MSP2_GET_OSD_WARNINGS
    0x3006, // MSP2_GET_TEXT
    0x3008, // MSP2_GET_LED_STRIP_CONFIG_VALUES
    0x300A, // MSP2_SENSOR_CONFIG_ACTIVE
    0x300C, // MSP2_MCU_INFO
    0x300D, // MSP2_GYRO_SENSOR_ACTIVE
];

/// True if `code` is a known read. `MSP_REBOOT` is handled separately by the connection
/// (it discards RAM rather than changing it) and is not a read.
pub fn msp_is_read_only(code: u16) -> bool {
    code != codes::MSP_REBOOT && MSP_READS.binary_search(&code).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_classification() {
        for l in [
            "",
            "   ",
            "# a comment",
            "version",
            "status",
            "serial",
            "resource",
            "feature",
            "aux",
            "dump all",
            "DUMP ALL",
            "diff all",
            "diff hardware",
            "get rx_spi_protocol",
            "get displayport_msp_col_adjust",
            "get",
        ] {
            assert!(cli_line_is_read_only(l), "{l:?} is read-only");
        }
        for l in [
            "batch start",
            "batch end",
            "save",
            "defaults nosave",
            "exit",
            "exit noreboot",
            "bl",
            "set acc_hardware = NONE",
            "serial 1 64 115200 57600 0 115200",
            "feature -RX_SERIAL",
            "aux 0 0 0 1700 2100 0 0",
            "resource MOTOR 1 B10",
            "profile 1",
            "align_board_roll 90",
            "nonsense",
        ] {
            assert!(!cli_line_is_read_only(l), "{l:?} may change settings");
        }
    }

    #[test]
    fn msp_classification() {
        assert!(MSP_READS.windows(2).all(|w| w[0] < w[1]), "sorted");
        for c in [
            codes::MSP_API_VERSION,
            codes::MSP_UID,
            codes::MSP_STATUS_EX,
            codes::MSP_RC,
            codes::MSP_ATTITUDE,
            codes::MSP2_COMMON_SERIAL_CONFIG,
            codes::MSP_BOARD_ALIGNMENT_CONFIG,
        ] {
            assert!(msp_is_read_only(c), "{c} is a read");
        }
        // MSP_SET_BOARD_ALIGNMENT_CONFIG, MSP_SET_FEATURE_CONFIG, MSP_SET_SENSOR_CONFIG,
        // MSP_EEPROM_WRITE, MSP_RESET_CONF, MSP_SELECT_SETTING, MSP_COPY_PROFILE,
        // MSP2_COMMON_SET_SERIAL_CONFIG, an unknown v2 code.
        for c in [39, 37, 97, 250, 208, 210, 183, 0x100A, 0x4242] {
            assert!(!msp_is_read_only(c), "{c} may write");
        }
        assert!(!msp_is_read_only(codes::MSP_REBOOT));
        assert!(!msp_is_read_only(codes::MSP_ACC_CALIBRATION));
    }
}
