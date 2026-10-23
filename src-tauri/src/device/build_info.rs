// SPDX-License-Identifier: GPL-3.0-or-later
//! MSP_BUILD_INFO build options and the `FcInfo.hdOsdSupported` decision (ARCHITECTURE §4.1
//! "Build options" / "HD OSD support").
//!
//! Firmware sources (betaflight tags 4.4.3, 4.5.5, 2025.12.5, 2026.6.2):
//! * `msp/msp.c` `case MSP_BUILD_INFO`: `buildDate[11] buildTime[8] shortGitRevision[7]`, then
//!   (4.5+, "Added in API version 1.46") `sbufWriteBuildInfoFlags()`. 4.4.3 stops after the
//!   git revision.
//! * `msp/msp_build_info.c`: one `sbufWriteU16` per compiled-in option, no count, no terminator.
//! * `msp/msp_build_info.h`: the option ids (same values as the Build API `/api/options` and
//!   Configurator `build_options.ts`). 2026.6.2 emits `BUILD_OPTION_FLIGHT_PLAN` twice when a
//!   build has both `USE_FLIGHT_PLAN` and `ENABLE_FLIGHT_PLAN`, so names are de-duplicated.
//! * `target/common_pre.h`: `USE_MSP_DISPLAYPORT` is defined inside `#if defined(USE_OSD)`
//!   on all four lines, for SD-only builds too, so `displayport_msp_col_adjust` (`cli/settings.c`,
//!   `#ifdef USE_MSP_DISPLAYPORT`) only proves "has an OSD", not "has HD".
//! * `io/displayport_msp.c` `displayPortMspInit`: `#ifndef USE_OSD_HD` forces
//!   `vcd_video_system` HD → AUTO, so HD needs `USE_OSD_HD`.
//! * `msp/msp.c` `case MSP_OSD_CANVAS`: two bytes `#ifdef USE_OSD`, else an empty reply.

use super::msp::{codes, ApiVersion, Reader};
use crate::device::error::Result;

/// `BUILD_OPTION_OSD_SD` (msp_build_info.h).
pub const BUILD_OPTION_OSD_SD: u16 = 16416;
/// `BUILD_OPTION_OSD_HD` (msp_build_info.h).
pub const BUILD_OPTION_OSD_HD: u16 = 16417;

const DATE_LEN: usize = 11;
const TIME_LEN: usize = 8;
const GIT_REV_LEN: usize = 7;
/// Bytes before the option ids.
pub const BUILD_INFO_HEADER_LEN: usize = DATE_LEN + TIME_LEN + GIT_REV_LEN;

/// Option id → Build API define. 2026.6.2 `msp_build_info.h` (a superset of 2025.12.5's, which
/// is a superset of 4.5.5's; 2026.6 adds 16427..=16429).
pub const BUILD_OPTION_NAMES: &[(u16, &str)] = &[
    (4097, "USE_SERIALRX_CRSF"),
    (4098, "USE_SERIALRX_FPORT"),
    (4099, "USE_SERIALRX_GHST"),
    (4100, "USE_SERIALRX_IBUS"),
    (4101, "USE_SERIALRX_JETIEXBUS"),
    (4102, "USE_RX_PPM"),
    (4103, "USE_SERIALRX_SBUS"),
    (4104, "USE_SERIALRX_SPEKTRUM"),
    (4105, "USE_SERIALRX_SRXL2"),
    (4106, "USE_SERIALRX_SUMD"),
    (4107, "USE_SERIALRX_SUMH"),
    (4108, "USE_SERIALRX_XBUS"),
    (4109, "USE_SERIALRX_MAVLINK"),
    (8230, "USE_BRUSHED"),
    (8231, "USE_DSHOT"),
    (8232, "USE_MULTISHOT"),
    (8233, "USE_ONESHOT"),
    (8234, "USE_PROSHOT"),
    (8235, "USE_PWM_OUTPUT"),
    (12301, "USE_TELEMETRY_FRSKY_HUB"),
    (12302, "USE_TELEMETRY_HOTT"),
    (12303, "USE_TELEMETRY_IBUS_EXTENDED"),
    (12304, "USE_TELEMETRY_LTM"),
    (12305, "USE_TELEMETRY_MAVLINK"),
    (12306, "USE_TELEMETRY_SMARTPORT"),
    (12307, "USE_TELEMETRY_SRXL"),
    (16404, "USE_ACRO_TRAINER"),
    (16405, "USE_AKK_SMARTAUDIO"),
    (16406, "USE_BATTERY_CONTINUE"),
    (16407, "USE_CAMERA_CONTROL"),
    (16408, "USE_DASHBOARD"),
    (16409, "USE_EMFAT_TOOLS"),
    (16410, "USE_ESCSERIAL_SIMONK"),
    (16411, "USE_FRSKYOSD"),
    (16412, "USE_GPS"),
    (16413, "USE_LED_STRIP"),
    (16414, "USE_LED_STRIP_64"),
    (16415, "USE_MAG"),
    (BUILD_OPTION_OSD_SD, "USE_OSD_SD"),
    (BUILD_OPTION_OSD_HD, "USE_OSD_HD"),
    (16418, "USE_PINIO"),
    (16419, "USE_RACE_PRO"),
    (16420, "USE_SERVOS"),
    (16421, "USE_VTX"),
    (16422, "USE_ALTITUDE_HOLD"),
    (16423, "USE_SOFTSERIAL"),
    (16424, "USE_WING"),
    (16425, "USE_POSITION_HOLD"),
    (16426, "USE_CHIRP"),
    // 2026.6.2
    (16427, "USE_FLIGHT_PLAN"),
    (16428, "USE_OPTICALFLOW"),
    (16429, "USE_RANGEFINDER"),
];

/// Decoded MSP_BUILD_INFO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// e.g. "Jan  1 2025"
    pub date: String,
    /// e.g. "00:00:00"
    pub time: String,
    /// 7 lowercase hex digits
    pub git_revision: String,
    /// Option ids in firmware order. `None` before API 1.46 (4.4), where the reply has none.
    pub option_ids: Option<Vec<u16>>,
}

impl BuildInfo {
    /// Build API defines for the known ids, in firmware order. Unknown ids (newer firmware
    /// than the table) are dropped with a log line; they are never passed on as raw numbers,
    /// so a cloud build never receives them.
    pub fn option_names(&self) -> Vec<String> {
        let ids = self.option_ids.iter().flatten();
        let unknown: Vec<u16> = ids
            .clone()
            .copied()
            .filter(|id| option_name(*id).is_none())
            .collect();
        if !unknown.is_empty() {
            log::info!("MSP_BUILD_INFO: dropping unknown build option ids {unknown:?}");
        }
        let mut names: Vec<String> = Vec::new();
        for name in ids.filter_map(|id| option_name(*id)) {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
        names
    }
}

pub fn option_name(id: u16) -> Option<&'static str> {
    BUILD_OPTION_NAMES
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| *n)
}

/// Parse MSP_BUILD_INFO. Option ids are read only for API 1.46+; like Configurator
/// (`MSPHelper.ts`) reading stops at the end of the payload or at a 0 id. A trailing odd byte
/// is ignored.
pub fn parse_build_info(p: &[u8], api: ApiVersion) -> Result<BuildInfo> {
    let mut r = Reader::new(codes::MSP_BUILD_INFO, p);
    let text = |b: &[u8]| b.iter().map(|&c| c as char).collect::<String>();
    let date = text(r.bytes(DATE_LEN)?);
    let time = text(r.bytes(TIME_LEN)?);
    let git_revision = text(r.bytes(GIT_REV_LEN)?);
    let option_ids = api.at_least(1, 46).then(|| {
        let mut ids = Vec::new();
        while r.remaining() >= 2 {
            match r.u16() {
                Ok(0) | Err(_) => break,
                Ok(id) => ids.push(id),
            }
        }
        ids
    });
    Ok(BuildInfo {
        date,
        time,
        git_revision,
        option_ids,
    })
}

/// Step 1: the build option list (4.5+). An empty list says nothing (every real build has at
/// least a motor protocol), so it stays undetermined.
pub fn hd_osd_from_build_options(ids: Option<&[u16]>) -> Option<bool> {
    let ids = ids.filter(|ids| !ids.is_empty())?;
    Some(ids.contains(&BUILD_OPTION_OSD_HD))
}

/// Step 2: MSP_OSD_CANVAS. Empty reply = no `USE_OSD` = no HD; 2 bytes = has an OSD, HD unknown.
pub fn hd_osd_from_osd_canvas(payload: &[u8]) -> Option<bool> {
    payload.is_empty().then_some(false)
}

/// Step 3: CLI `get displayport_msp_col_adjust` output. `INVALID NAME` = no MSP DisplayPort
/// (= no OSD) = no HD. The setting existing is not evidence of HD (SD-only builds have it).
pub fn hd_osd_from_cli_get(get_text: &str) -> Option<bool> {
    get_text
        .to_ascii_uppercase()
        .contains("INVALID NAME")
        .then_some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V44: ApiVersion = ApiVersion {
        protocol: 0,
        major: 1,
        minor: 45,
    };
    const V45: ApiVersion = ApiVersion {
        protocol: 0,
        major: 1,
        minor: 46,
    };
    const V2025: ApiVersion = ApiVersion {
        protocol: 0,
        major: 1,
        minor: 47,
    };

    fn header() -> Vec<u8> {
        let p = b"Jan  1 202500:00:00abc1234".to_vec();
        assert_eq!(p.len(), BUILD_INFO_HEADER_LEN);
        p
    }

    fn with_ids(ids: &[u16]) -> Vec<u8> {
        let mut p = header();
        for id in ids {
            p.extend(id.to_le_bytes());
        }
        p
    }

    /// Decision chain as `MspClient::identify` + `apply_cli_probe` run it.
    fn decide(build: Option<&BuildInfo>, canvas: Option<&[u8]>, cli: Option<&str>) -> Option<bool> {
        hd_osd_from_build_options(build.and_then(|b| b.option_ids.as_deref()))
            .or_else(|| canvas.and_then(hd_osd_from_osd_canvas))
            .or_else(|| cli.and_then(hd_osd_from_cli_get))
    }

    #[test]
    fn parses_4_4_build_info_without_options() {
        // 4.4.3 msp.c: date, time, git revision only. Even if bytes followed, API 1.45 has none.
        let b = parse_build_info(&header(), V44).unwrap();
        assert_eq!(b.date, "Jan  1 2025");
        assert_eq!(b.time, "00:00:00");
        assert_eq!(b.git_revision, "abc1234");
        assert_eq!(b.option_ids, None);
        assert!(b.option_names().is_empty());
        let b = parse_build_info(&with_ids(&[BUILD_OPTION_OSD_HD]), V44).unwrap();
        assert_eq!(b.option_ids, None);
        assert!(parse_build_info(&header()[..20], V44).is_err());
    }

    #[test]
    fn parses_4_5_and_2025_12_option_ids() {
        // A typical cloud build: CRSF, GPS, OSD SD+HD, VTX, DSHOT, plus an id we don't know.
        let ids = [4097, 16412, 16416, 16417, 16421, 8231, 40000];
        for api in [V45, V2025] {
            let b = parse_build_info(&with_ids(&ids), api).unwrap();
            assert_eq!(b.option_ids.as_deref(), Some(&ids[..]));
            assert_eq!(
                b.option_names(),
                [
                    "USE_SERIALRX_CRSF",
                    "USE_GPS",
                    "USE_OSD_SD",
                    "USE_OSD_HD",
                    "USE_VTX",
                    "USE_DSHOT"
                ]
            );
        }
        // Unknown ids are dropped, never sent on as numbers.
        let names = parse_build_info(&with_ids(&[40000, 65535]), V2025)
            .unwrap()
            .option_names();
        assert!(names.is_empty(), "{names:?}");
        // Trailing odd byte and a 0 id stop the list.
        let mut p = with_ids(&[8231]);
        p.push(0x7f);
        assert_eq!(
            parse_build_info(&p, V45).unwrap().option_ids,
            Some(vec![8231])
        );
        let p = with_ids(&[8231, 0, 16417]);
        assert_eq!(
            parse_build_info(&p, V45).unwrap().option_ids,
            Some(vec![8231])
        );
        // Header only on 1.46+: empty list.
        assert_eq!(
            parse_build_info(&header(), V45).unwrap().option_ids,
            Some(vec![])
        );
    }

    #[test]
    fn names_table_matches_firmware_ids() {
        assert_eq!(option_name(16417), Some("USE_OSD_HD"));
        assert_eq!(option_name(16416), Some("USE_OSD_SD"));
        assert_eq!(option_name(4097), Some("USE_SERIALRX_CRSF"));
        assert_eq!(option_name(8235), Some("USE_PWM_OUTPUT"));
        assert_eq!(option_name(1), None);
        let mut ids: Vec<u16> = BUILD_OPTION_NAMES.iter().map(|(i, _)| *i).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), BUILD_OPTION_NAMES.len(), "duplicate id");
    }

    #[test]
    fn decision_4_5_and_2025_12_from_build_options() {
        for api in [V45, V2025] {
            let hd = parse_build_info(&with_ids(&[8231, 16416, 16417]), api).unwrap();
            assert_eq!(decide(Some(&hd), None, None), Some(true));
            // Build options are authoritative: an OSD canvas or the setting don't override.
            let sd_only = parse_build_info(&with_ids(&[8231, 16416]), api).unwrap();
            assert_eq!(
                decide(
                    Some(&sd_only),
                    Some(&[30, 16]),
                    Some("displayport_msp_col_adjust = -1")
                ),
                Some(false)
            );
            let no_osd = parse_build_info(&with_ids(&[8231, 4097]), api).unwrap();
            assert_eq!(decide(Some(&no_osd), None, None), Some(false));
            // Empty list: falls through to the other evidence.
            let empty = parse_build_info(&header(), api).unwrap();
            assert_eq!(decide(Some(&empty), Some(&[53, 20]), None), None);
            assert_eq!(decide(Some(&empty), Some(&[]), None), Some(false));
        }
    }

    #[test]
    fn decision_4_4_from_canvas_and_cli() {
        let b = parse_build_info(&header(), V44).unwrap();
        // OSD build (canvas answered), setting present: SD-only builds look the same → null.
        let get_ok = "displayport_msp_col_adjust = 0\r\nAllowed range: -6 - 0";
        assert_eq!(decide(Some(&b), Some(&[53, 20]), Some(get_ok)), None);
        assert_eq!(decide(Some(&b), Some(&[30, 16]), None), None);
        // No USE_OSD: empty canvas reply.
        assert_eq!(decide(Some(&b), Some(&[]), None), Some(false));
        // CLI says the setting does not exist (4.4/4.5/2025.12 cli.c error format).
        let get_err = "###ERROR IN get: INVALID NAME: displayport_msp_col_adjust###";
        assert_eq!(
            decide(Some(&b), Some(&[30, 16]), Some(get_err)),
            Some(false)
        );
        // Other CLI errors are not evidence.
        assert_eq!(
            hd_osd_from_cli_get("###ERROR: UNKNOWN COMMAND 'get'###"),
            None
        );
        // Nothing read at all.
        assert_eq!(decide(None, None, None), None);
    }
}
