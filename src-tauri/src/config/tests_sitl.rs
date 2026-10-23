//! Tests against real Betaflight SITL captures (`tests/fixtures/sitl/<version>/`):
//! MSP decoding, `dump all` parsing, and MockFc conformance with real CLI responses.

use std::path::PathBuf;

use super::firmware::Dialect;
use super::mock::MockFc;
use super::msp;
use super::snapshot::parse_dump;
use crate::device::Connection;

const VERSIONS: [&str; 4] = ["4.4.3", "4.5.5", "2025.12.5", "2026.6.2"];

fn sitl(version: &str, file: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sitl")
        .join(version)
        .join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// `NAME code hex` lines from `msp.txt`.
fn msp_capture(version: &str, name: &str) -> Vec<u8> {
    let text = sitl(version, "msp.txt");
    let line = text
        .lines()
        .find(|l| l.split_whitespace().next() == Some(name))
        .unwrap_or_else(|| panic!("{version}: no {name}"));
    let hex = line.split_whitespace().nth(2).unwrap_or("");
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn msp_decoders_read_real_sitl_replies() {
    for (v, api) in VERSIONS.iter().zip(["1.45", "1.46", "1.47", "1.48"]) {
        assert_eq!(
            msp::decode_api_version(&msp_capture(v, "API_VERSION")).as_deref(),
            Some(api)
        );
        assert_eq!(
            msp::decode_fc_variant(&msp_capture(v, "FC_VARIANT")).as_deref(),
            Some("BTFL")
        );
        assert_eq!(
            msp::decode_fc_version(&msp_capture(v, "FC_VERSION")).as_deref(),
            Some(*v)
        );
        let b = msp::decode_board_info(&msp_capture(v, "BOARD_INFO")).unwrap();
        assert_eq!(b.identifier, "SITL", "{v}");
        assert_eq!(b.target_name, "SITL", "{v}");
        assert_eq!(b.board_name, "", "{v}: SITL has no board config");
        // 2026.6 reports MCU_TYPE_ID_PROVIDED_BY_NAME; the name is in MSP2_MCU_INFO.
        let want = if v.starts_with("2026") {
            msp::MCU_TYPE_ID_PROVIDED_BY_NAME
        } else {
            0 // SIMULATOR
        };
        assert_eq!(b.mcu_type_id, Some(want), "{v}");
        // MSP_UID renders exactly like the CLI `mcu_id` of the same build.
        let dump = sitl(v, "dump_all.txt");
        let mcu_id = dump
            .lines()
            .find_map(|l| l.strip_prefix("mcu_id "))
            .unwrap()
            .trim();
        assert_eq!(
            msp::decode_uid(&msp_capture(v, "UID")).as_deref(),
            Some(mcu_id),
            "{v}"
        );
    }
}

#[test]
fn mock_msp_matches_real_layouts() {
    for (v, dir) in VERSIONS.iter().zip(["4.4", "4.5", "2025.12", "2026.6"]) {
        let mut fc = MockFc::load(dir, "speedybeef405v4");
        let mut q = |c| fc.msp_request(c, &[]).unwrap();
        let (api, var, ver, board, uid) = (
            q(msp::MSP_API_VERSION),
            q(msp::MSP_FC_VARIANT),
            q(msp::MSP_FC_VERSION),
            q(msp::MSP_BOARD_INFO),
            q(msp::MSP_UID),
        );
        // Same framing as the real replies (lengths differ only by the strings).
        assert_eq!(api, msp_capture(v, "API_VERSION"), "{v}");
        assert_eq!(ver, msp_capture(v, "FC_VERSION"), "{v}");
        let real = msp_capture(v, "BOARD_INFO");
        let pad = "SPEEDYBEEF405V4".len() + "SPBE".len() + "STM32F405".len() - "SITL".len();
        assert_eq!(board.len(), real.len() + pad, "{v}");
        assert_eq!(uid.len(), 12);
        let info = msp::fc_info(&api, &var, &ver, &board, Some(&uid)).unwrap();
        assert_eq!(info.firmware_version, *v);
        assert_eq!(info.board_name, "SPEEDYBEEF405V4");
        assert_eq!(info.manufacturer_id.as_deref(), Some("SPBE"));
        assert_eq!(info.mcu.as_deref(), Some("STM32F405"));
        assert_eq!(info.uid.as_deref(), Some(fc.mcu_id.as_str()));
        assert!(Dialect::from_version(&info.firmware_version).is_ok());
    }
}

#[test]
fn parse_dump_reads_real_sitl_dump_all() {
    for v in VERSIONS {
        let s = parse_dump(&sitl(v, "dump_all.txt"));
        assert_eq!(s.version.as_deref(), Some(v));
        assert_eq!(
            s.dialect().map(|d| d.as_str().to_string()),
            Dialect::from_version(v)
                .ok()
                .map(|d| d.as_str().to_string())
        );
        // SITL has no USB VCP; its MSP port is UART1 (TCP 5761).
        let u1 = s.ports.iter().find(|p| p.name == "UART1").unwrap();
        assert_eq!(u1.function_mask, 1, "{v}: {:?}", s.ports);
        assert_eq!(s.ports.len(), 8, "{v}");
        assert!(s.settings.len() > 300, "{v}: {} settings", s.settings.len());
        assert_eq!(s.setting("craft_name"), Some("-"), "{v}");
        assert_eq!(s.features.get("ANTI_GRAVITY"), Some(&true), "{v}");
        // 2025.12 renamed the idle setting to `motor_idle` and compiles it without DSHOT;
        // 4.x `dshot_idle_value` only exists with USE_DSHOT, which SITL lacks.
        let v2025 = !v.starts_with("4.");
        assert_eq!(s.setting("motor_idle").is_some(), v2025, "{v}");
        assert!(s.setting("dshot_idle_value").is_none(), "{v}");
    }
}

/// `>>> cmd` blocks: (command, response lines without the echo, blanks and prompts).
fn transcript(v: &str, file: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for l in sitl(v, file).lines() {
        if let Some(c) = l.strip_prefix(">>> ") {
            out.push((c.to_string(), Vec::new()));
        } else if let Some((c, r)) = out.last_mut() {
            let t = l.trim();
            if !(t.is_empty() || t == "#" || t == c || l.starts_with("---")) {
                r.push(t.to_string());
            }
        }
    }
    out
}

fn mock_lines(fc: &mut MockFc, cmd: &str) -> Vec<String> {
    let r = fc.cli_exec(cmd).unwrap();
    let mut it = r.lines().map(str::trim).filter(|l| !l.is_empty());
    assert_eq!(it.next(), Some(cmd), "MockFc must echo the command");
    it.map(str::to_string).collect()
}

#[test]
fn mock_cli_answers_like_real_sitl() {
    for v in VERSIONS {
        let mut fc = MockFc::from_dump(&sitl(v, "dump_all.txt"));
        if !v.starts_with("2026") {
            // compiled out of SITL before 2026.6 (its SITL target enables GPS + flight plan)
            fc.unavailable_features = vec!["GPS".into()];
        }
        fc.enter_cli().unwrap();
        for (cmd, real) in transcript(v, "transcript.txt") {
            if cmd == "resource" {
                continue; // not compiled into SITL; present on every real F4/F7 build
            }
            let mock = mock_lines(&mut fc, &cmd);
            // `get` also prints "Allowed values"/"Default value"/"String length" lines,
            // which MockFc does not know; the first line carries the value or the error.
            let n = if cmd.starts_with("get ") || real.iter().any(|l| l.starts_with("Allowed")) {
                1
            } else {
                real.len()
            };
            assert_eq!(&mock[..mock.len().min(n)], &real[..n], "{v}: `{cmd}`");
        }
    }
}

#[test]
fn mock_save_semantics_match_real_sitl() {
    for v in VERSIONS {
        let mut fc = MockFc::from_dump(&sitl(v, "dump_all.txt"));
        fc.enter_cli().unwrap();
        let mut saves = 0;
        for (cmd, real) in transcript(v, "save.txt") {
            if cmd == "save" || cmd == "exit" {
                fc.exit_cli(cmd == "save").unwrap();
                fc.enter_cli().unwrap();
                saves += usize::from(cmd == "save");
                continue;
            }
            let mock = mock_lines(&mut fc, &cmd);
            let n = if cmd.starts_with("get ") {
                1
            } else {
                real.len()
            };
            assert_eq!(
                &mock[..mock.len().min(n)],
                &real[..n.min(real.len())],
                "{v}: `{cmd}`"
            );
        }
        assert!(saves >= 2, "{v}");
        assert_eq!(
            fc.saves_after_error, 1,
            "{v}: a failed batch is still saved"
        );
    }
}

#[test]
fn real_sitl_diff_all_replays_through_the_guard() {
    for v in VERSIONS {
        let lines = super::guard::replay_lines(&sitl(v, "diff_all.txt"))
            .unwrap_or_else(|e| panic!("{v}: {e}"));
        let v2026 = v.starts_with("2026");
        // SITL defaults: GPS is off before 2026.6 (so `feature -GPS` is in the diff) and on
        // in the 2026.6 SITL target; 2026.6 also lists its 3 battery profiles.
        let mut want: Vec<String> = [
            "defaults nosave",
            "feature -GPS",
            "profile 0",
            "profile 1",
            "profile 2",
            "profile 3",
            "profile 0",
            "rateprofile 0",
            "rateprofile 1",
            "rateprofile 2",
            "rateprofile 3",
            "rateprofile 0",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if v2026 {
            want.retain(|l| l != "feature -GPS");
            let rp = want.len() - 1; // before "rateprofile 0" (restore)
            for (i, l) in [
                "battery_profile 0",
                "battery_profile 1",
                "battery_profile 2",
            ]
            .into_iter()
            .enumerate()
            {
                want.insert(rp + i, l.into());
            }
            want.push("battery_profile 0".into());
        }
        assert_eq!(lines, want, "{v}");
    }
}
