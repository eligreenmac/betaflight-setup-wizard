//! Per-firmware catalogs: beeper / DShot beacon conditions, OSD elements and the stable
//! mode names (`config_firmware_catalog`), and the beeper name mapping the generator uses.
//!
//! Beeper sources: `io/beeper.c` `beeperTable` and `io/beeper.h` `BEEPER_ALLOWED_MODES` /
//! `DSHOT_BEACON_ALLOWED_MODES` at 4.4.3, 4.5.5 and 2025.12.5.

use super::firmware::Dialect;
use super::{modes, osd};
use crate::model::{BeeperName, DshotBeaconCondition, FirmwareCatalog};

/// Stable names in `beeperTable` order (the order `dump all` prints, and the order the
/// CLI's prefix match walks; shorter names come first, so full names always match exactly).
pub const BEEPERS: [BeeperName; 24] = [
    BeeperName::GyroCalibrated,
    BeeperName::RxLost,
    BeeperName::RxLostLanding,
    BeeperName::Disarming,
    BeeperName::Arming,
    BeeperName::ArmingGpsFix,
    BeeperName::ArmingGpsNoFix,
    BeeperName::BatCritLow,
    BeeperName::BatLow,
    BeeperName::GpsStatus,
    BeeperName::RxSet,
    BeeperName::AccCalibration,
    BeeperName::AccCalibrationFail,
    BeeperName::ReadyBeep,
    BeeperName::MultiBeeps,
    BeeperName::DisarmRepeat,
    BeeperName::Armed,
    BeeperName::SystemInit,
    BeeperName::OnUsb,
    BeeperName::BlackboxErase,
    BeeperName::CrashFlip,
    BeeperName::CamConnectionOpen,
    BeeperName::CamConnectionClose,
    BeeperName::RcSmoothingInitFail,
];

/// The CLI name of a beeper condition on `dialect`; None = not on that line.
/// 2025.12 renamed CRASH_FLIP to CRASHFLIP and removed RC_SMOOTHING_INIT_FAIL.
pub fn beeper_cli_name(b: BeeperName, dialect: Dialect) -> Option<&'static str> {
    use BeeperName::*;
    let v2025 = dialect >= Dialect::V2025_12;
    Some(match b {
        GyroCalibrated => "GYRO_CALIBRATED",
        RxLost => "RX_LOST",
        RxLostLanding => "RX_LOST_LANDING",
        Disarming => "DISARMING",
        Arming => "ARMING",
        ArmingGpsFix => "ARMING_GPS_FIX",
        ArmingGpsNoFix => "ARMING_GPS_NO_FIX",
        BatCritLow => "BAT_CRIT_LOW",
        BatLow => "BAT_LOW",
        GpsStatus => "GPS_STATUS",
        RxSet => "RX_SET",
        AccCalibration => "ACC_CALIBRATION",
        AccCalibrationFail => "ACC_CALIBRATION_FAIL",
        ReadyBeep => "READY_BEEP",
        MultiBeeps => "MULTI_BEEPS",
        DisarmRepeat => "DISARM_REPEAT",
        Armed => "ARMED",
        SystemInit => "SYSTEM_INIT",
        OnUsb => "ON_USB",
        BlackboxErase => "BLACKBOX_ERASE",
        CrashFlip if v2025 => "CRASHFLIP",
        CrashFlip => "CRASH_FLIP",
        CamConnectionOpen => "CAM_CONNECTION_OPEN",
        CamConnectionClose => "CAM_CONNECTION_CLOSE",
        RcSmoothingInitFail if v2025 => return None,
        RcSmoothingInitFail => "RC_SMOOTHING_INIT_FAIL",
    })
}

/// DShot beacon conditions in `beeperTable` order (RX_LOST is index 1, RX_SET index 10).
pub const BEACONS: [DshotBeaconCondition; 2] =
    [DshotBeaconCondition::RxLost, DshotBeaconCondition::RxSet];

pub fn beacon_cli_name(b: DshotBeaconCondition) -> &'static str {
    match b {
        DshotBeaconCondition::RxLost => "RX_LOST",
        DshotBeaconCondition::RxSet => "RX_SET",
    }
}

/// `config_firmware_catalog`: what the firmware line offers (independent of the board's
/// build options; the generator checks the FC's own `dump all` at plan time).
pub fn firmware_catalog(dialect: Dialect) -> FirmwareCatalog {
    FirmwareCatalog {
        beepers: BEEPERS
            .into_iter()
            .filter(|b| beeper_cli_name(*b, dialect).is_some())
            .collect(),
        beacon_events: BEACONS.to_vec(),
        osd_elements: osd::elements(dialect).map(|(n, _)| n.to_string()).collect(),
        modes: modes::names(dialect).map(str::to_string).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beeper_names_per_line() {
        // the serde (stable) name is the 4.4 / 4.5 CLI name
        for b in BEEPERS {
            let stable = serde_json::to_value(b).unwrap();
            assert_eq!(stable.as_str(), beeper_cli_name(b, Dialect::V4_5));
            assert_eq!(
                beeper_cli_name(b, Dialect::V4_4),
                beeper_cli_name(b, Dialect::V4_5)
            );
        }
        assert_eq!(
            beeper_cli_name(BeeperName::CrashFlip, Dialect::V2025_12),
            Some("CRASHFLIP")
        );
        assert_eq!(
            beeper_cli_name(BeeperName::RcSmoothingInitFail, Dialect::V2025_12),
            None
        );
        // no full name is a prefix of an earlier one (the CLI takes the first prefix match)
        for d in Dialect::ALL {
            let names: Vec<&str> = BEEPERS
                .iter()
                .filter_map(|b| beeper_cli_name(*b, d))
                .collect();
            for (i, n) in names.iter().enumerate() {
                assert!(!names[..i].iter().any(|e| e.starts_with(n)), "{n}");
            }
        }
    }

    #[test]
    fn catalog_per_line() {
        let c = firmware_catalog(Dialect::V4_4);
        assert_eq!((c.beepers.len(), c.osd_elements.len()), (24, 77));
        assert!(!c.modes.iter().any(|m| m == "ALTHOLD"));
        let c = firmware_catalog(Dialect::V4_5);
        assert_eq!((c.beepers.len(), c.osd_elements.len()), (24, 80));
        let c = firmware_catalog(Dialect::V2025_12);
        assert_eq!((c.beepers.len(), c.osd_elements.len()), (23, 82));
        assert!(c.modes.iter().any(|m| m == "ALTHOLD"));
        assert!(c.modes.iter().any(|m| m == "FLIP OVER AFTER CRASH"));
        assert_eq!(c.beacon_events, BEACONS.to_vec());
        let j = serde_json::to_value(&c).unwrap();
        assert!(j["beaconEvents"].is_array() && j["osdElements"].is_array());
        assert_eq!(j["beepers"][20], "CRASH_FLIP");
    }
}
