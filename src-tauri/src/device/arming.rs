// SPDX-License-Identifier: GPL-3.0-or-later
//! Arming-disable flags (MSP_STATUS_EX `armingDisableFlags`), decoded per firmware line.
//!
//! The bit layout is Betaflight `fc/runtime_config.h` `armingDisableFlags_e`, and the names
//! are `fc/runtime_config.c` `armingDisableFlagNames` (what CLI `status` prints). Bits 0..=24
//! keep their meaning on every supported line. The tail differs by line:
//!
//! | line (API)      | 25          | 26          | 27          | 28           | 29         |
//! |-----------------|-------------|-------------|-------------|--------------|------------|
//! | 4.4 / 4.5 (45/46) | ARM_SWITCH  | –           | –           | –            | –          |
//! | 2025.12 (47)    | FLIP_SWITCH | ALT_HOLD_SW | POS_HOLD_SW | ARM_SWITCH   | –          |
//! | 2026.6 (48+)    | FLIP_SWITCH | ALT_HOLD_SW | POS_HOLD_SW | AUTOPILOT_SW | ARM_SWITCH |
//!
//! Also, bit 3 is `BADRX` on 4.4 and `NOT_DISARMED` from 4.5, and bit 20 is `RPMFILTER` on
//! 4.4 and `DSHOT_TELEM` from 4.5. Sources: tags 4.4.3, 4.5.5, 2025.12.5, 2026.6.2.

use crate::device::msp::ApiVersion;

const NAMES_4_4: &[&str] = &[
    "NOGYRO",
    "FAILSAFE",
    "RXLOSS",
    "BADRX",
    "BOXFAILSAFE",
    "RUNAWAY",
    "CRASH",
    "THROTTLE",
    "ANGLE",
    "BOOTGRACE",
    "NOPREARM",
    "LOAD",
    "CALIB",
    "CLI",
    "CMS",
    "BST",
    "MSP",
    "PARALYZE",
    "GPS",
    "RESCUE_SW",
    "RPMFILTER",
    "REBOOT_REQD",
    "DSHOT_BBANG",
    "NO_ACC_CAL",
    "MOTOR_PROTO",
    "ARMSWITCH",
];

const NAMES_4_5: &[&str] = &[
    "NOGYRO",
    "FAILSAFE",
    "RXLOSS",
    "NOT_DISARMED",
    "BOXFAILSAFE",
    "RUNAWAY",
    "CRASH",
    "THROTTLE",
    "ANGLE",
    "BOOTGRACE",
    "NOPREARM",
    "LOAD",
    "CALIB",
    "CLI",
    "CMS",
    "BST",
    "MSP",
    "PARALYZE",
    "GPS",
    "RESCUE_SW",
    "DSHOT_TELEM",
    "REBOOT_REQD",
    "DSHOT_BBANG",
    "NO_ACC_CAL",
    "MOTOR_PROTO",
    "ARMSWITCH",
];

const NAMES_2025_12: &[&str] = &[
    "NOGYRO",
    "FAILSAFE",
    "RXLOSS",
    "NOT_DISARMED",
    "BOXFAILSAFE",
    "RUNAWAY",
    "CRASH",
    "THROTTLE",
    "ANGLE",
    "BOOTGRACE",
    "NOPREARM",
    "LOAD",
    "CALIB",
    "CLI",
    "CMS",
    "BST",
    "MSP",
    "PARALYZE",
    "GPS",
    "RESCUE_SW",
    "DSHOT_TELEM",
    "REBOOT_REQD",
    "DSHOT_BBANG",
    "NO_ACC_CAL",
    "MOTOR_PROTO",
    "FLIP_SWITCH",
    "ALT_HOLD_SW",
    "POS_HOLD_SW",
    "ARM_SWITCH",
];

const NAMES_2026_6: &[&str] = &[
    "NOGYRO",
    "FAILSAFE",
    "RXLOSS",
    "NOT_DISARMED",
    "BOXFAILSAFE",
    "RUNAWAY",
    "CRASH",
    "THROTTLE",
    "ANGLE",
    "BOOTGRACE",
    "NOPREARM",
    "LOAD",
    "CALIB",
    "CLI",
    "CMS",
    "BST",
    "MSP",
    "PARALYZE",
    "GPS",
    "RESCUE_SW",
    "DSHOT_TELEM",
    "REBOOT_REQD",
    "DSHOT_BBANG",
    "NO_ACC_CAL",
    "MOTOR_PROTO",
    "FLIP_SWITCH",
    "ALT_HOLD_SW",
    "POS_HOLD_SW",
    "AUTOPILOT_SW",
    "ARM_SWITCH",
];

/// Bit 12, `ARMING_DISABLED_CALIBRATING`: the same on every line.
pub const CALIBRATING_BIT: u8 = 12;

/// `armingDisableFlagNames` for the firmware line that reports `api`. Index = bit.
pub fn flag_names(api: ApiVersion) -> &'static [&'static str] {
    if api.at_least(1, 48) {
        NAMES_2026_6
    } else if api.at_least(1, 47) {
        NAMES_2025_12
    } else if api.at_least(1, 46) {
        NAMES_4_5
    } else {
        NAMES_4_4
    }
}

/// Bit of `ARMING_DISABLED_ARM_SWITCH` (always the last flag): 25 on 4.4/4.5, 28 on 2025.12,
/// 29 on 2026.6. It is set together with any other flag while the arm switch is on.
pub fn arm_switch_bit(api: ApiVersion) -> u8 {
    (flag_names(api).len() - 1) as u8
}

/// Names of the set flags (lowest bit first) and the set bits this line does not define.
pub fn decode(flags: u32, api: ApiVersion) -> (Vec<&'static str>, u32) {
    let names = flag_names(api);
    let mut out = Vec::new();
    let mut known = 0u32;
    for (bit, name) in names.iter().enumerate() {
        if flags & (1 << bit) != 0 {
            out.push(*name);
            known |= 1 << bit;
        }
    }
    (out, flags & !known)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(minor: u8) -> ApiVersion {
        ApiVersion {
            protocol: 0,
            major: 1,
            minor,
        }
    }

    #[test]
    fn table_lengths_match_the_firmware() {
        // ARMING_DISABLE_FLAGS_COUNT per line.
        assert_eq!(flag_names(api(45)).len(), 26);
        assert_eq!(flag_names(api(46)).len(), 26);
        assert_eq!(flag_names(api(47)).len(), 29);
        assert_eq!(flag_names(api(48)).len(), 30);
        assert_eq!(flag_names(api(49)).len(), 30, "newer API: latest table");
        for m in [45, 46, 47, 48] {
            assert_eq!(flag_names(api(m))[CALIBRATING_BIT as usize], "CALIB");
            assert_eq!(flag_names(api(m))[24], "MOTOR_PROTO");
        }
    }

    #[test]
    fn arm_switch_moves_on_2026_6() {
        assert_eq!(arm_switch_bit(api(45)), 25);
        assert_eq!(arm_switch_bit(api(46)), 25);
        assert_eq!(arm_switch_bit(api(47)), 28);
        assert_eq!(arm_switch_bit(api(48)), 29);
        // Bit 28: ARM_SWITCH on 2025.12, AUTOPILOT_SW on 2026.6.
        assert_eq!(decode(1 << 28, api(47)), (vec!["ARM_SWITCH"], 0));
        assert_eq!(decode(1 << 28, api(48)), (vec!["AUTOPILOT_SW"], 0));
        assert_eq!(decode(1 << 29, api(48)), (vec!["ARM_SWITCH"], 0));
        // Bit 29 is unknown before 2026.6; bits 26-28 are unknown on 4.5.
        assert_eq!(decode(1 << 29, api(47)), (vec![], 1 << 29));
        assert_eq!(
            decode((1 << 25) | (1 << 27), api(46)),
            (vec!["ARMSWITCH"], 1 << 27)
        );
    }

    #[test]
    fn decodes_flags_seen_on_sitl() {
        // MSP_STATUS_EX flags from the SITL smoke runs (no RX, fresh boot).
        assert_eq!(
            decode(772, api(48)),
            (vec!["RXLOSS", "ANGLE", "BOOTGRACE"], 0)
        );
        assert_eq!(
            decode(4868, api(47)),
            (vec!["RXLOSS", "ANGLE", "BOOTGRACE", "CALIB"], 0)
        );
        assert_eq!(
            decode(16777476, api(45)),
            (vec!["RXLOSS", "ANGLE", "MOTOR_PROTO"], 0)
        );
        // 4.4 vs 4.5 renames.
        assert_eq!(decode(1 << 3, api(45)).0, ["BADRX"]);
        assert_eq!(decode(1 << 3, api(46)).0, ["NOT_DISARMED"]);
        assert_eq!(decode(1 << 20, api(45)).0, ["RPMFILTER"]);
        assert_eq!(decode(1 << 20, api(48)).0, ["DSHOT_TELEM"]);
    }
}
