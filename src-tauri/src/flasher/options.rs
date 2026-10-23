// SPDX-License-Identifier: GPL-3.0-or-later
//! Build options for cloud builds (ARCHITECTURE §6 "Build options"): which Build API option
//! values the wizard's answers need, and how the board's current options are resolved against
//! a release's option list (`/api/options/{release}`).
//!
//! Option values are the Build API's `value` strings (e.g. `USE_SERIALRX_CRSF`). They are the
//! same defines the firmware reports in MSP_BUILD_INFO (`FcInfo.buildOptions`).

use crate::flasher::build_types::{ApiBuildOption, BuildOptions};
use crate::flasher::error::{FlashError, Result};
use crate::model::{FirmwareRequirements, OsdVideoSystem, ReceiverKind, RxProtocol};
use std::collections::BTreeSet;

/// Build API option for a receiver. `None` = nothing to add: SPI receivers (ExpressLRS,
/// FrSky) come from the board's target config, not a Build API option; MSP / NONE need none.
pub fn rx_option(kind: ReceiverKind, protocol: Option<RxProtocol>) -> Option<&'static str> {
    match kind {
        ReceiverKind::Ppm => Some("USE_RX_PPM"),
        ReceiverKind::Serial => Some(match protocol? {
            RxProtocol::Crsf => "USE_SERIALRX_CRSF",
            RxProtocol::Ghst => "USE_SERIALRX_GHST",
            RxProtocol::Sbus => "USE_SERIALRX_SBUS",
            RxProtocol::Fport => "USE_SERIALRX_FPORT",
            RxProtocol::Ibus => "USE_SERIALRX_IBUS",
            RxProtocol::Srxl2 => "USE_SERIALRX_SRXL2",
            RxProtocol::Spek1024 | RxProtocol::Spek2048 => "USE_SERIALRX_SPEKTRUM",
            RxProtocol::Sumd => "USE_SERIALRX_SUMD",
            RxProtocol::Expresslrs
            | RxProtocol::FrskyD
            | RxProtocol::FrskyX
            | RxProtocol::FrskyXLbt => return None,
        }),
        ReceiverKind::Spi | ReceiverKind::Msp | ReceiverKind::None => None,
    }
}

/// Build API option for the OSD video system: HD needs `USE_OSD_HD` (MSP DisplayPort
/// canvas), an analog OSD (PAL / NTSC / AUTO) `USE_OSD_SD`.
pub fn osd_option(video_system: OsdVideoSystem) -> &'static str {
    match video_system {
        OsdVideoSystem::Hd => "USE_OSD_HD",
        OsdVideoSystem::Pal | OsdVideoSystem::Ntsc | OsdVideoSystem::Auto => "USE_OSD_SD",
    }
}

/// The options the wizard's answers need (`FirmwareRequest.requiredOptions`). The names are
/// the same on 4.4.3, 4.5.x and 2025.12.x; use [`check_required`] for a specific release.
pub fn required_options(req: &FirmwareRequirements) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(rx) = &req.receiver {
        out.extend(rx_option(rx.kind, rx.protocol));
    }
    if let Some(vs) = req.osd.as_ref().and_then(|o| o.video_system) {
        out.push(osd_option(vs));
    }
    out.into_iter().map(str::to_string).collect()
}

fn all(list: &BuildOptions) -> impl Iterator<Item = &ApiBuildOption> {
    list.radio_protocols
        .iter()
        .chain(&list.telemetry_protocols)
        .chain(&list.motor_protocols)
        .chain(&list.general_options)
}

/// Every option value the release offers (the empty "none" telemetry entry excluded).
pub fn offered(list: &BuildOptions) -> BTreeSet<String> {
    all(list)
        .map(|o| o.value.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

/// The release's default selection (what Configurator preselects without a board).
pub fn defaults(list: &BuildOptions) -> Vec<String> {
    all(list)
        .filter(|o| o.default && !o.value.trim().is_empty())
        .map(|o| o.value.trim().to_string())
        .collect()
}

/// `REQUIRED_OPTION_UNSUPPORTED` if `release` does not offer every required option.
pub fn check_required(release: &str, required: &[String], list: &BuildOptions) -> Result<()> {
    let offered = offered(list);
    let missing: Vec<String> = required
        .iter()
        .map(|o| o.trim())
        .filter(|o| !o.is_empty() && !offered.contains(*o))
        .map(str::to_string)
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(FlashError::RequiredOptionUnsupported {
            release: release.to_string(),
            options: missing,
        })
    }
}

/// Options to send for a build, plus the board options left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub options: Vec<String>,
    pub dropped: Vec<String>,
}

/// Resolve the request against the release's option list: base = the board's options (or
/// the release's defaults when the board reported none), filtered to what the release
/// offers; required options are always added and must be offered.
pub fn resolve(
    release: &str,
    board: &[String],
    required: &[String],
    list: &BuildOptions,
) -> Result<Resolved> {
    check_required(release, required, list)?;
    let offered = offered(list);
    let base: Vec<String> = if board.iter().all(|o| o.trim().is_empty()) {
        defaults(list)
    } else {
        board.iter().map(|o| o.trim().to_string()).collect()
    };
    let mut options = Vec::new();
    let mut dropped = Vec::new();
    for o in base.into_iter().filter(|o| !o.is_empty()) {
        let into = if offered.contains(&o) {
            &mut options
        } else {
            &mut dropped
        };
        if !into.contains(&o) {
            into.push(o);
        }
    }
    for r in required.iter().map(|o| o.trim()).filter(|o| !o.is_empty()) {
        if !options.iter().any(|o| o == r) {
            options.push(r.to_string());
        }
    }
    if !dropped.is_empty() {
        log::info!("build options not offered by {release}, left out: {dropped:?}");
    }
    Ok(Resolved { options, dropped })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{OsdRequirement, ReceiverRequirement};

    pub(crate) const OPTIONS_4_4_3: &str =
        include_str!("../../tests/fixtures/build-api/options-4.4.3.json");
    pub(crate) const OPTIONS_4_5_2: &str =
        include_str!("../../tests/fixtures/build-api/options-4.5.2.json");
    pub(crate) const OPTIONS_2025_12_2: &str =
        include_str!("../../tests/fixtures/build-api/options-2025.12.2.json");
    pub(crate) const OPTIONS_2026_6_2: &str =
        include_str!("../../tests/fixtures/build-api/options-2026.6.2.json");

    fn list(json: &str) -> BuildOptions {
        serde_json::from_str(json).unwrap()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// A 2025.12 board: every option the 2025.12.2 list offers.
    fn board_2025_12() -> Vec<String> {
        offered(&list(OPTIONS_2025_12_2)).into_iter().collect()
    }

    #[test]
    fn options_from_2025_12_sent_to_older_releases_are_filtered() {
        let board = board_2025_12();
        let r = resolve("4.5.2", &board, &[], &list(OPTIONS_4_5_2)).unwrap();
        assert_eq!(
            r.dropped,
            s(&[
                "USE_ALTITUDE_HOLD",
                "USE_POSITION_HOLD",
                "USE_SOFTSERIAL",
                "USE_WING"
            ])
        );
        assert!(r.options.contains(&"USE_RACE_PRO".to_string()));
        assert_eq!(r.options.len() + r.dropped.len(), board.len());

        let r = resolve("4.4.3", &board, &[], &list(OPTIONS_4_4_3)).unwrap();
        assert_eq!(
            r.dropped,
            s(&[
                "USE_ALTITUDE_HOLD",
                "USE_POSITION_HOLD",
                "USE_RACE_PRO",
                "USE_SOFTSERIAL",
                "USE_WING"
            ])
        );
        // Same release: nothing dropped.
        let r = resolve("2025.12.2", &board, &[], &list(OPTIONS_2025_12_2)).unwrap();
        assert!(r.dropped.is_empty());
        assert_eq!(r.options, board);
    }

    #[test]
    fn required_options_are_added_even_if_the_board_lacks_them() {
        let board = s(&["USE_SERIALRX_CRSF", "USE_DSHOT", "USE_OSD_SD"]);
        let r = resolve(
            "4.5.2",
            &board,
            &s(&["USE_OSD_HD", "USE_SERIALRX_SBUS", "USE_OSD_SD"]),
            &list(OPTIONS_4_5_2),
        )
        .unwrap();
        assert_eq!(
            r.options,
            s(&[
                "USE_SERIALRX_CRSF",
                "USE_DSHOT",
                "USE_OSD_SD",
                "USE_OSD_HD",
                "USE_SERIALRX_SBUS"
            ])
        );
        assert!(r.dropped.is_empty());
    }

    #[test]
    fn required_option_missing_from_the_release_is_refused() {
        let e = resolve(
            "4.4.3",
            &s(&["USE_DSHOT"]),
            &s(&["USE_OSD_HD", "USE_RACE_PRO", "USE_WING"]),
            &list(OPTIONS_4_4_3),
        )
        .unwrap_err();
        assert!(
            matches!(&e, FlashError::RequiredOptionUnsupported { release, options }
                if release == "4.4.3" && *options == s(&["USE_RACE_PRO", "USE_WING"])),
            "{e:?}"
        );
        let v = serde_json::to_value(crate::error::AppError::from(e)).unwrap();
        assert_eq!(v["code"], "REQUIRED_OPTION_UNSUPPORTED");
        assert_eq!(v["details"]["options"][1], "USE_WING");
    }

    #[test]
    fn unknown_and_raw_numeric_board_options_are_dropped() {
        let r = resolve(
            "2025.12.2",
            &s(&["USE_DSHOT", "16999", "USE_FUTURE_THING", ""]),
            &[],
            &list(OPTIONS_2025_12_2),
        )
        .unwrap();
        assert_eq!(r.options, s(&["USE_DSHOT"]));
        assert_eq!(r.dropped, s(&["16999", "USE_FUTURE_THING"]));
    }

    #[test]
    fn empty_board_options_use_the_release_defaults() {
        let r = resolve(
            "4.4.3",
            &[],
            &s(&["USE_SERIALRX_SBUS"]),
            &list(OPTIONS_4_4_3),
        )
        .unwrap();
        assert_eq!(
            r.options,
            s(&[
                "USE_SERIALRX_CRSF",
                "USE_DSHOT",
                "USE_ACRO_TRAINER",
                "USE_GPS",
                "USE_LED_STRIP",
                "USE_OPTICALFLOW",
                "USE_OSD_SD",
                "USE_OSD_HD",
                "USE_PINIO",
                "USE_RANGEFINDER",
                "USE_VTX",
                "USE_SERIALRX_SBUS"
            ])
        );
        assert!(r.dropped.is_empty());
    }

    #[test]
    fn requirements_map_to_build_api_names_offered_by_every_release() {
        let req = |kind, protocol, vs| FirmwareRequirements {
            receiver: Some(ReceiverRequirement { kind, protocol }),
            osd: Some(OsdRequirement { video_system: vs }),
        };
        use OsdVideoSystem as V;
        use ReceiverKind as K;
        use RxProtocol as P;
        assert_eq!(
            required_options(&req(K::Serial, Some(P::Crsf), Some(V::Hd))),
            s(&["USE_SERIALRX_CRSF", "USE_OSD_HD"])
        );
        assert_eq!(
            required_options(&req(K::Serial, Some(P::Spek2048), Some(V::Pal))),
            s(&["USE_SERIALRX_SPEKTRUM", "USE_OSD_SD"])
        );
        assert_eq!(
            required_options(&req(K::Spi, Some(P::Expresslrs), None)),
            Vec::<String>::new()
        );
        assert_eq!(
            required_options(&req(K::Ppm, None, None)),
            s(&["USE_RX_PPM"])
        );
        assert_eq!(
            required_options(&FirmwareRequirements::default()),
            Vec::<String>::new()
        );
        // The DroneModel itself deserializes as requirements (extra fields ignored).
        let m: FirmwareRequirements = serde_json::from_str(
            r#"{"schemaVersion":1,"receiver":{"kind":"SERIAL","protocol":"SBUS","port":"UART2","telemetry":true},
                "osd":{"videoSystem":"NTSC","elements":[]},"vtx":{"kind":"NONE"}}"#,
        )
        .unwrap();
        assert_eq!(
            required_options(&m),
            s(&["USE_SERIALRX_SBUS", "USE_OSD_SD"])
        );

        // Every name the mapping can produce is offered by 4.4.3, 4.5.2, 2025.12.2 and 2026.6.2.
        let mut names: Vec<&str> = vec![osd_option(V::Hd), osd_option(V::Pal)];
        for p in [
            P::Crsf,
            P::Ghst,
            P::Sbus,
            P::Fport,
            P::Ibus,
            P::Srxl2,
            P::Spek1024,
            P::Spek2048,
            P::Sumd,
        ] {
            names.extend(rx_option(K::Serial, Some(p)));
        }
        names.extend(rx_option(K::Ppm, None));
        for json in [
            OPTIONS_4_4_3,
            OPTIONS_4_5_2,
            OPTIONS_2025_12_2,
            OPTIONS_2026_6_2,
        ] {
            let offered = offered(&list(json));
            for n in &names {
                assert!(offered.contains(*n), "{n} not offered");
            }
        }
    }

    #[test]
    fn options_2026_6_2_parse_and_extend_2025_12() {
        let l = list(OPTIONS_2026_6_2);
        assert!(
            l.extra.is_empty(),
            "no unknown top-level lists: {:?}",
            l.extra.keys()
        );
        assert_eq!(l.radio_protocols.len(), 13);
        let new = offered(&l);
        let old = offered(&list(OPTIONS_2025_12_2));
        assert!(old.is_subset(&new), "2026.6.2 removed {:?}", &old - &new);
        assert_eq!(
            new.difference(&old).cloned().collect::<Vec<_>>(),
            s(&["USE_FLIGHT_PLAN"])
        );
        for o in [
            "USE_RX_PPM",
            "USE_SERIALRX_SPEKTRUM",
            "USE_SERIALRX_CRSF",
            "USE_OSD_HD",
            "USE_OSD_SD",
        ] {
            assert!(new.contains(o), "{o}");
        }
        let osd: Vec<_> = l
            .general_options
            .iter()
            .filter(|o| o.group.as_deref() == Some("OSD"))
            .map(|o| o.value.as_str())
            .collect();
        assert_eq!(osd, ["USE_OSD_SD", "USE_OSD_HD", "USE_FRSKYOSD"]);

        // A 2026.6 board re-flashed to 2025.12.2 only loses USE_FLIGHT_PLAN.
        let board: Vec<String> = new.into_iter().collect();
        let r = resolve("2025.12.2", &board, &[], &list(OPTIONS_2025_12_2)).unwrap();
        assert_eq!(r.dropped, s(&["USE_FLIGHT_PLAN"]));
        let r = resolve("2026.6.2", &board, &[], &l).unwrap();
        assert!(r.dropped.is_empty());
    }

    /// Every Build API option key of 2026.6.2 decodes to the same define via the
    /// MSP_BUILD_INFO table (`device::build_info`), so a 2026.6 board's options survive.
    #[test]
    fn options_2026_6_2_keys_match_the_build_info_table() {
        let v: serde_json::Value = serde_json::from_str(OPTIONS_2026_6_2).unwrap();
        let mut n = 0;
        for group in v.as_object().unwrap().values() {
            for o in group.as_array().unwrap() {
                let (Some(key), Some(value)) = (o["key"].as_u64(), o["value"].as_str()) else {
                    continue;
                };
                assert_eq!(
                    crate::device::build_info::option_name(key as u16),
                    Some(value),
                    "key {key}"
                );
                n += 1;
            }
        }
        assert!(n >= 50, "{n}");
    }
}
