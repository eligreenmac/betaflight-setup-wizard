//! OSD elements per firmware line, the `osd_*_pos` encoding and config's default layout.
//!
//! Sources: Betaflight `src/main` at 4.4.3, 4.5.5 and 2025.12.5: `osd/osd.h` (encoding,
//! `osd_items_e`), `cli/settings.c` (`osd_<name>_pos`), `osd/osd.c` (firmware defaults).

use super::firmware::Dialect;
use crate::model::{OsdLayout, OsdLayoutElement, OsdVideoSystem};

/// (stable name = `osd_items_e` without `OSD_`, CLI name in `osd_<cli>_pos`, first line).
/// CLI names are identical on all three lines; later lines only add elements.
/// Order: `cli/settings.c`.
pub const ELEMENTS: &[(&str, &str, Dialect)] = &[
    ("MAIN_BATT_VOLTAGE", "vbat", Dialect::V4_4),
    ("RSSI_VALUE", "rssi", Dialect::V4_4),
    ("LINK_QUALITY", "link_quality", Dialect::V4_4),
    ("TX_UPLINK_POWER", "link_tx_power", Dialect::V4_4),
    ("RSSI_DBM_VALUE", "rssi_dbm", Dialect::V4_4),
    ("RSNR_VALUE", "rsnr", Dialect::V4_4),
    ("ITEM_TIMER_1", "tim_1", Dialect::V4_4),
    ("ITEM_TIMER_2", "tim_2", Dialect::V4_4),
    (
        "REMAINING_TIME_ESTIMATE",
        "remaining_time_estimate",
        Dialect::V4_4,
    ),
    ("FLYMODE", "flymode", Dialect::V4_4),
    ("ANTI_GRAVITY", "anti_gravity", Dialect::V4_4),
    ("G_FORCE", "g_force", Dialect::V4_4),
    ("THROTTLE_POS", "throttle", Dialect::V4_4),
    ("VTX_CHANNEL", "vtx_channel", Dialect::V4_4),
    ("CROSSHAIRS", "crosshairs", Dialect::V4_4),
    ("HORIZON_SIDEBARS", "ah_sbar", Dialect::V4_4),
    ("ARTIFICIAL_HORIZON", "ah", Dialect::V4_4),
    ("CURRENT_DRAW", "current", Dialect::V4_4),
    ("MAH_DRAWN", "mah_drawn", Dialect::V4_4),
    ("WATT_HOURS_DRAWN", "wh_drawn", Dialect::V4_4),
    ("MOTOR_DIAG", "motor_diag", Dialect::V4_4),
    ("CRAFT_NAME", "craft_name", Dialect::V4_4),
    ("PILOT_NAME", "pilot_name", Dialect::V4_4),
    ("GPS_SPEED", "gps_speed", Dialect::V4_4),
    ("GPS_LON", "gps_lon", Dialect::V4_4),
    ("GPS_LAT", "gps_lat", Dialect::V4_4),
    ("GPS_LAP_TIME_CURRENT", "gps_lap_curr", Dialect::V4_5),
    ("GPS_LAP_TIME_PREVIOUS", "gps_lap_prev", Dialect::V4_5),
    ("GPS_LAP_TIME_BEST3", "gps_lap_best3", Dialect::V4_5),
    ("GPS_SATS", "gps_sats", Dialect::V4_4),
    ("HOME_DIR", "home_dir", Dialect::V4_4),
    ("HOME_DIST", "home_dist", Dialect::V4_4),
    ("FLIGHT_DIST", "flight_dist", Dialect::V4_4),
    ("COMPASS_BAR", "compass_bar", Dialect::V4_4),
    ("ALTITUDE", "altitude", Dialect::V4_4),
    ("ROLL_PIDS", "pid_roll", Dialect::V4_4),
    ("PITCH_PIDS", "pid_pitch", Dialect::V4_4),
    ("YAW_PIDS", "pid_yaw", Dialect::V4_4),
    ("DEBUG", "debug", Dialect::V4_4),
    ("DEBUG2", "debug2", Dialect::V2025_12),
    ("POWER", "power", Dialect::V4_4),
    ("PIDRATE_PROFILE", "pidrate_profile", Dialect::V4_4),
    ("WARNINGS", "warnings", Dialect::V4_4),
    ("AVG_CELL_VOLTAGE", "avg_cell_voltage", Dialect::V4_4),
    ("PITCH_ANGLE", "pit_ang", Dialect::V4_4),
    ("ROLL_ANGLE", "rol_ang", Dialect::V4_4),
    ("MAIN_BATT_USAGE", "battery_usage", Dialect::V4_4),
    ("DISARMED", "disarmed", Dialect::V4_4),
    ("NUMERICAL_HEADING", "nheading", Dialect::V4_4),
    ("UP_DOWN_REFERENCE", "up_down_reference", Dialect::V4_4),
    ("READY_MODE", "ready_mode", Dialect::V4_4),
    ("NUMERICAL_VARIO", "nvario", Dialect::V4_4),
    ("ESC_TMP", "esc_tmp", Dialect::V4_4),
    ("ESC_RPM", "esc_rpm", Dialect::V4_4),
    ("ESC_RPM_FREQ", "esc_rpm_freq", Dialect::V4_4),
    ("RTC_DATETIME", "rtc_date_time", Dialect::V4_4),
    ("ADJUSTMENT_RANGE", "adjustment_range", Dialect::V4_4),
    ("FLIP_ARROW", "flip_arrow", Dialect::V4_4),
    ("CORE_TEMPERATURE", "core_temp", Dialect::V4_4),
    ("LOG_STATUS", "log_status", Dialect::V4_4),
    ("STICK_OVERLAY_LEFT", "stick_overlay_left", Dialect::V4_4),
    ("STICK_OVERLAY_RIGHT", "stick_overlay_right", Dialect::V4_4),
    ("RATE_PROFILE_NAME", "rate_profile_name", Dialect::V4_4),
    ("PID_PROFILE_NAME", "pid_profile_name", Dialect::V4_4),
    ("PROFILE_NAME", "profile_name", Dialect::V4_4),
    ("RC_CHANNELS", "rcchannels", Dialect::V4_4),
    ("CAMERA_FRAME", "camera_frame", Dialect::V4_4),
    ("EFFICIENCY", "efficiency", Dialect::V4_4),
    ("TOTAL_FLIGHTS", "total_flights", Dialect::V4_4),
    ("AUX_VALUE", "aux", Dialect::V4_4),
    ("SYS_GOGGLE_VOLTAGE", "sys_goggle_voltage", Dialect::V4_4),
    ("SYS_VTX_VOLTAGE", "sys_vtx_voltage", Dialect::V4_4),
    ("SYS_BITRATE", "sys_bitrate", Dialect::V4_4),
    ("SYS_DELAY", "sys_delay", Dialect::V4_4),
    ("SYS_DISTANCE", "sys_distance", Dialect::V4_4),
    ("SYS_LQ", "sys_lq", Dialect::V4_4),
    ("SYS_GOGGLE_DVR", "sys_goggle_dvr", Dialect::V4_4),
    ("SYS_VTX_DVR", "sys_vtx_dvr", Dialect::V4_4),
    ("SYS_WARNINGS", "sys_warnings", Dialect::V4_4),
    ("SYS_VTX_TEMP", "sys_vtx_temp", Dialect::V4_4),
    ("SYS_FAN_SPEED", "sys_fan_speed", Dialect::V4_4),
    ("LIDAR_DIST", "lidar_dist", Dialect::V2025_12),
];

/// Elements that exist on `dialect`, in settings order.
pub fn elements(dialect: Dialect) -> impl Iterator<Item = (&'static str, &'static str)> {
    ELEMENTS
        .iter()
        .filter(move |(_, _, since)| dialect >= *since)
        .map(|(n, c, _)| (*n, *c))
}

/// CLI name for a stable element name on `dialect` (case-insensitive; an `OSD_` prefix is
/// accepted). Err = not an element of that firmware line.
pub fn cli_name(name: &str, dialect: Dialect) -> Result<&'static str, String> {
    let key = name.trim().to_ascii_uppercase();
    let key = key.strip_prefix("OSD_").unwrap_or(&key);
    match ELEMENTS.iter().find(|(n, _, _)| *n == key) {
        Some((_, cli, since)) if dialect >= *since => Ok(cli),
        Some((n, _, since)) => Err(format!(
            "OSD element {n} needs Betaflight {} or newer",
            since.as_str()
        )),
        None => Err(format!(
            "unknown OSD element '{name}' for Betaflight {}",
            dialect.as_str()
        )),
    }
}

/// `osd/osd.h` (identical on 4.4 / 4.5 / 2025.12): x bits 0-4 and bit 10 (x <= 63),
/// y bits 5-9, bits 11-13 = visible in OSD profile 1/2/3, bits 14-15 = element type.
pub const PROFILE_BITS: u16 = 0x3800;
pub const TYPE_BITS: u16 = 0xC000;

pub fn encode(x: u8, y: u8, enabled: bool, current: u16) -> u16 {
    let (x, y) = (u16::from(x), u16::from(y));
    (x & 0x1F)
        | ((x << 5) & 0x400)
        | ((y & 0x1F) << 5)
        | if enabled { PROFILE_BITS } else { 0 }
        | (current & TYPE_BITS)
}

/// (x, y, visible in any profile)
pub fn decode(pos: u16) -> (u8, u8, bool) {
    let x = (pos & 0x1F) | ((pos & 0x400) >> 5);
    let y = (pos >> 5) & 0x1F;
    (x as u8, y as u8, pos & PROFILE_BITS != 0)
}

/// Config's default (x, y, visible) for `name` on a `cols` x `rows` grid. The firmware's
/// own defaults hide every element at the centre, so the wizard owns a usable layout:
/// link and battery on top, warnings and craft name centred at the bottom, flight mode
/// and timer in the bottom corners; everything else hidden at a sensible spot.
pub fn default_position(name: &str, cols: u8, rows: u8) -> (u8, u8, bool) {
    let (mc, mr) = (cols / 2, (rows - 1) / 2);
    let right = cols - 7;
    match name {
        "LINK_QUALITY" => (1, 1, true),
        "MAIN_BATT_VOLTAGE" => (right, 1, true),
        "MAH_DRAWN" => (right, 3, true),
        "WARNINGS" => (mc - 6, rows - 4, true),
        "FLYMODE" => (1, rows - 2, true),
        "ITEM_TIMER_2" => (right, rows - 2, true),
        "CRAFT_NAME" => (mc - 6, rows - 1, true),
        "VTX_CHANNEL" => (1, 0, false),
        "RSSI_DBM_VALUE" => (1, 2, false),
        "RSSI_VALUE" => (1, 3, false),
        "TX_UPLINK_POWER" => (1, 4, false),
        "GPS_SATS" => (1, 5, false),
        "GPS_SPEED" => (1, 6, false),
        "AVG_CELL_VOLTAGE" => (right, 2, false),
        "CURRENT_DRAW" => (right - 1, 4, false),
        "ALTITUDE" => (right - 1, 5, false),
        "NUMERICAL_VARIO" => (right - 1, 6, false),
        "COMPASS_BAR" => (mc - 4, 0, false),
        "HOME_DIR" => (mc, 1, false),
        "HOME_DIST" => (mc - 3, 2, false),
        "CROSSHAIRS" => (mc - 2, mr - 1, false),
        "ARTIFICIAL_HORIZON" => (mc - 1, mr - 5, false),
        "HORIZON_SIDEBARS" => (mc - 1, mr - 1, false),
        "DISARMED" => (mc - 4, rows - 6, false),
        "THROTTLE_POS" => (1, rows - 3, false),
        "ITEM_TIMER_1" => (right, rows - 3, false),
        _ => (mc - 5, mr, false),
    }
}

/// `config_osd_default_layout`: every element of `dialect` at config's default position.
pub fn default_layout(video_system: OsdVideoSystem, dialect: Dialect) -> OsdLayout {
    let (cols, rows) = video_system.grid();
    OsdLayout {
        video_system,
        cols,
        rows,
        elements: elements(dialect)
            .map(|(name, _)| {
                let (x, y, enabled) = default_position(name, cols, rows);
                OsdLayoutElement {
                    name: name.to_string(),
                    x,
                    y,
                    enabled,
                }
            })
            .collect(),
    }
}

/// `vcd_video_system` value.
pub fn video_system_name(v: OsdVideoSystem) -> &'static str {
    match v {
        OsdVideoSystem::Auto => "AUTO",
        OsdVideoSystem::Pal => "PAL",
        OsdVideoSystem::Ntsc => "NTSC",
        OsdVideoSystem::Hd => "HD",
    }
}

pub fn parse_video_system(s: &str) -> Option<OsdVideoSystem> {
    [
        OsdVideoSystem::Auto,
        OsdVideoSystem::Pal,
        OsdVideoSystem::Ntsc,
        OsdVideoSystem::Hd,
    ]
    .into_iter()
    .find(|v| video_system_name(*v).eq_ignore_ascii_case(s.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_VS: [OsdVideoSystem; 4] = [
        OsdVideoSystem::Auto,
        OsdVideoSystem::Pal,
        OsdVideoSystem::Ntsc,
        OsdVideoSystem::Hd,
    ];

    #[test]
    fn element_counts_per_line() {
        assert_eq!(elements(Dialect::V4_4).count(), 77);
        assert_eq!(elements(Dialect::V4_5).count(), 80);
        assert_eq!(elements(Dialect::V2025_12).count(), 82);
        assert_eq!(cli_name("main_batt_voltage", Dialect::V4_4), Ok("vbat"));
        assert_eq!(cli_name("OSD_ITEM_TIMER_2", Dialect::V4_5), Ok("tim_2"));
        assert!(cli_name("GPS_LAP_TIME_BEST3", Dialect::V4_4).is_err());
        assert_eq!(
            cli_name("GPS_LAP_TIME_BEST3", Dialect::V4_5),
            Ok("gps_lap_best3")
        );
        assert!(cli_name("LIDAR_DIST", Dialect::V4_5).is_err());
        assert!(cli_name("DEBUG2", Dialect::V2025_12).is_ok());
        assert!(cli_name("NOPE", Dialect::V2025_12).is_err());
    }

    /// Values computed by hand from OSD_POS in osd/osd.h (and the firmware defaults the
    /// composed fixtures carry: 4.4 SD centre (10,7) = 234, 4.5 HD centre (21,10) = 341,
    /// WARNINGS (20,13) in all profiles = 14772).
    #[test]
    fn position_encoding() {
        assert_eq!(encode(10, 7, false, 0), 234);
        assert_eq!(encode(21, 10, false, 0), 341);
        assert_eq!(encode(20, 13, true, 0), 14772);
        // x >= 32 uses bit 10
        assert_eq!(encode(46, 1, true, 0), 14 | 0x400 | 32 | 0x3800);
        assert_eq!(decode(encode(52, 19, true, 0)), (52, 19, true));
        // type bits are kept from the FC, profile bits are replaced
        assert_eq!(encode(1, 1, false, 0xC000 | 0x0800 | 33), 0xC000 | 33);
        for x in 0..64u8 {
            for y in 0..32u8 {
                assert_eq!(decode(encode(x, y, x % 2 == 0, 0)), (x, y, x % 2 == 0));
            }
        }
    }

    #[test]
    fn default_layout_fits_every_grid() {
        for d in Dialect::ALL {
            for vs in ALL_VS {
                let l = default_layout(vs, d);
                assert_eq!(l.elements.len(), elements(d).count());
                let mut seen = std::collections::BTreeSet::new();
                for e in &l.elements {
                    assert!(e.x < l.cols && e.y < l.rows, "{vs:?} {}", e.name);
                    if e.enabled {
                        assert!(seen.insert((e.x, e.y)), "{vs:?} {} overlaps", e.name);
                    }
                }
                assert_eq!(l.elements.iter().filter(|e| e.enabled).count(), 7);
            }
        }
        let hd = default_layout(OsdVideoSystem::Hd, Dialect::V4_5);
        assert_eq!((hd.cols, hd.rows), (53, 20));
        let v = hd
            .elements
            .iter()
            .find(|e| e.name == "MAIN_BATT_VOLTAGE")
            .unwrap();
        assert_eq!((v.x, v.y, v.enabled), (46, 1, true));
    }
}
