//! `config_restore_after_flash` against MockFc: goldens per line pair (pure plan), full
//! restores per line pair, refusals and link failures (ARCHITECTURE §5.2).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use super::backup::BackupStore;
use super::commands::{restore_after_flash, LinkSource};
use super::firmware::Dialect;
use super::mock::{fixture, LinkFault, MockFc};
use super::restore::{backup_body, mask_secrets, plan_restore, Step, Target};
use crate::device::FcLink;
use crate::error::{AppError, AppResult};
use crate::model::{ConfigProgress, ConfigStage, RestoreReport, RestoreSkipReason as R};

const FLAT_MOCK_READBACK: &[&str] = &[
    "Read back: p_roll is 55, restored 50",
    "Read back: battery_profile_name is 4S850, restored 4S1500",
    "Read back: bat_capacity is 850, restored 1500",
    "Read back: vbat_max_cell_voltage is 440, restored 435",
];

const UID: &str = "000000000000000100000002";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn full_version(d: Dialect) -> &'static str {
    match d {
        Dialect::V4_4 => "4.4.3",
        Dialect::V4_5 => "4.5.5",
        Dialect::V2025_12 => "2025.12.5",
        Dialect::V2026_6 => "2026.6.2",
    }
}

/// Hand-built `diff all` of a used SPEEDYBEEF405V4 per line (tests/fixtures/restore).
fn backup_text(d: Dialect) -> String {
    let p = fixtures().join(format!("restore/{}.diff_all.txt", d.as_str()));
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn non_empty_lines(body: &str) -> usize {
    body.lines().filter(|l| !l.trim().is_empty()).count()
}

struct Source(Mutex<Option<MockFc>>);

impl Source {
    fn new(fc: MockFc) -> Self {
        Source(Mutex::new(Some(fc)))
    }
    fn fc<T>(&self, f: impl FnOnce(&mut MockFc) -> T) -> T {
        f(self.0.lock().unwrap().as_mut().unwrap())
    }
}

impl LinkSource for Source {
    fn with_link<T>(&self, f: impl FnOnce(&mut dyn FcLink) -> AppResult<T>) -> AppResult<T> {
        let mut g = self.0.lock().unwrap();
        match g.as_mut() {
            Some(fc) => f(fc),
            None => Err(AppError::NotConnected),
        }
    }
}

struct Run {
    src: Source,
    result: AppResult<RestoreReport>,
    progress: Vec<ConfigProgress>,
    path: String,
}

fn store(tag: &str) -> BackupStore {
    let dir = std::env::temp_dir().join(format!("bfsw-restore-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    BackupStore::new(dir)
}

/// Save `diff` as a backup of `uid`/`board` on `version`, then restore it onto `fc`.
fn run_with(
    tag: &str,
    diff: &str,
    uid: &str,
    board: &str,
    version: &str,
    fc: Option<MockFc>,
) -> Run {
    let st = store(tag);
    let bk = st
        .save(
            diff,
            uid,
            board,
            version,
            UNIX_EPOCH + Duration::from_secs(1_792_746_900),
        )
        .unwrap();
    let src = Source(Mutex::new(fc));
    let mut progress = Vec::new();
    let result = restore_after_flash(&src, &st, &bk.id, &mut |p| progress.push(p));
    let _ = std::fs::remove_dir_all(st.root());
    Run {
        src,
        result,
        progress,
        path: bk.path,
    }
}

fn restore(tag: &str, from: Dialect, fc: MockFc) -> Run {
    run_with(
        tag,
        &backup_text(from),
        UID,
        "SPEEDYBEEF405V4",
        full_version(from),
        Some(fc),
    )
}

fn board(d: Dialect) -> MockFc {
    MockFc::load(d.as_str(), "speedybeef405v4")
}

fn skipped(r: &RestoreReport, reason: R) -> Vec<&str> {
    r.skipped
        .iter()
        .filter(|s| s.reason == reason)
        .map(|s| s.line.as_str())
        .collect()
}

fn reason_of(e: &AppError) -> (String, serde_json::Value) {
    assert_eq!(e.code(), "CONFIG", "{e:?}");
    let d = e.details().expect("details");
    (d["reason"].as_str().unwrap_or_default().to_string(), d)
}

/// Common invariants of a successful restore.
fn check_ok(run: &Run, from: Dialect, to: Dialect) -> RestoreReport {
    let r = run
        .result
        .as_ref()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .clone();
    let body = backup_text(from);
    assert_eq!(
        r.applied + r.failed.len() + r.skipped.len(),
        non_empty_lines(&body),
        "every backup line is counted once"
    );
    assert!(r.saved);
    assert_eq!(r.line_changed, from != to);
    assert_eq!(r.from_version, full_version(from));
    assert_eq!(r.to_version, full_version(to));
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains("41,42"), "secret leaked: {json}");
    for p in &run.progress {
        let m = p.message.clone().unwrap_or_default();
        assert!(!m.contains("expresslrs") && !m.contains("set "), "{m}");
    }
    let stages: Vec<ConfigStage> = run.progress.iter().map(|p| p.stage).collect();
    assert_eq!(stages.first(), Some(&ConfigStage::Restoring));
    let last_restoring = run
        .progress
        .iter()
        .rfind(|p| p.stage == ConfigStage::Restoring)
        .unwrap();
    assert_eq!(last_restoring.line, last_restoring.total);
    assert!(stages.ends_with(&[
        ConfigStage::Saving,
        ConfigStage::Rebooting,
        ConfigStage::Reconnected
    ]));
    run.src.fc(|fc| {
        let from_batch: Vec<&str> = fc
            .log
            .iter()
            .map(String::as_str)
            .skip_while(|l| *l != "batch start")
            .collect();
        let save = from_batch
            .iter()
            .position(|l| *l == "save")
            .expect("save sent");
        let sent = &from_batch[..=save];
        assert_eq!(&sent[..2], &["batch start", "defaults nosave"]);
        let n = sent.len();
        assert_eq!(&sent[n - 2..], &["batch end", "save"]);
        // Read-back after the reboot.
        assert_eq!(&from_batch[save + 1..], &["version", "dump all"]);
        assert_eq!(fc.saves_refused, 0);
        assert_eq!(fc.reboots, 1);
        // Never a board identity line, never a second `defaults`/`save`/`batch`.
        for l in &sent[2..n - 2] {
            for bad in [
                "board_name",
                "mcu_id",
                "signature",
                "manufacturer_id",
                "save",
                "batch",
                "defaults",
            ] {
                assert!(!l.starts_with(bad), "sent `{l}`");
            }
        }
    });
    // MockFc keeps one flat settings map (no per-profile values), so the backup's two PID
    // profiles (p_roll 50 / 55) read back as the last one written; a real FC keeps both.
    for w in r.warnings.iter().filter(|w| w.starts_with("Read back")) {
        assert!(
            FLAT_MOCK_READBACK.contains(&w.as_str()),
            "{w}: {:?}",
            r.warnings
        );
    }
    r
}

// ------------------------------------------------------------------ goldens

/// Plans for every line pair (4 x 4) against the target board's `dump all`.
/// Regenerate with `UPDATE_GOLDEN=1 cargo test restore_golden`.
#[test]
fn restore_golden_plans_per_line_pair() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();
    for from in Dialect::ALL {
        for to in Dialect::ALL {
            let body = backup_text(from);
            let target = Target::from_dump(to, &fixture(to.as_str(), "speedybeef405v4"));
            let plan = plan_restore(&body, from, &target);
            assert_eq!(plan.steps.len(), non_empty_lines(&body));
            let text = format!(
                "# golden: restore plan, backup {} -> board {} (speedybeef405v4); generated by config::restore, do not edit\n{}",
                from.as_str(),
                to.as_str(),
                plan.render()
            );
            assert!(!text.contains("41,42"));
            let path = fixtures().join(format!(
                "golden/restore/{}_to_{}.txt",
                from.as_str(),
                to.as_str()
            ));
            if update {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &text).unwrap();
            } else if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                failures.push(path.display().to_string());
            }
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatch (UPDATE_GOLDEN=1 to regenerate): {failures:?}"
    );
}

#[test]
fn same_line_plans_send_everything_but_identity_and_session() {
    for d in Dialect::ALL {
        let body = backup_text(d);
        let target = Target::from_dump(d, &fixture(d.as_str(), "speedybeef405v4"));
        let plan = plan_restore(&body, d, &target);
        for s in &plan.steps {
            match s {
                Step::Send { line, cmd, .. } => {
                    assert_eq!(line, cmd, "{d:?}: same line is verbatim")
                }
                Step::Skip { reason, line, .. } => assert!(
                    matches!(
                        reason,
                        R::Comment | R::SessionControl | R::BoardIdentity | R::NotInBuild
                    ),
                    "{d:?}: {line} {reason:?}"
                ),
            }
        }
    }
}

#[test]
fn secrets_are_masked() {
    assert_eq!(
        mask_secrets("set expresslrs_uid = 1,2,3"),
        "set expresslrs_uid = ***"
    );
    assert_eq!(
        mask_secrets("###ERROR IN set: INVALID NAME: expresslrs_uid = 1,2,3###"),
        "###ERROR IN set: INVALID NAME: expresslrs_uid = ***###"
    );
    assert_eq!(mask_secrets("set p_roll = 45"), "set p_roll = 45");
    assert_eq!(
        backup_body("# bfsw-backup v1\n# a: b\n\ndiff all\n\nx"),
        "diff all\n\nx"
    );
}

// ------------------------------------------------------------------ full restores

#[test]
fn restore_2025_12_to_2026_6() {
    let (from, to) = (Dialect::V2025_12, Dialect::V2026_6);
    let run = restore("a", from, board(to));
    let r = check_ok(&run, from, to);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    assert!(skipped(&r, R::HardwareLineChanged).contains(&"resource LED_STRIP 1 A08"));
    assert!(skipped(&r, R::RemovedOnTarget).contains(&"set abs_control_gain = 5"));
    assert!(
        skipped(&r, R::SemanticsChanged).is_empty(),
        "same D-term meaning"
    );
    // mag_hardware QMC5883 -> QMC5883L (2026.6 renamed the driver).
    run.src.fc(|fc| {
        let s = fc.snapshot();
        assert_eq!(s.setting("mag_hardware"), Some("QMC5883L"));
        assert_eq!(s.setting("motor_idle"), Some("450"));
        assert_eq!(s.setting("bat_capacity"), Some("1500"));
        assert_eq!(s.setting("craft_name"), Some("RESTORE"));
        assert!(fc
            .log
            .iter()
            .any(|l| l == "serial UART1 2 115200 57600 0 115200"));
        assert!(fc.log.iter().any(|l| l == "aux 4 3 3 900 1300 0 0"));
    });
    assert!(
        r.warnings.iter().any(|w| w.contains("battery profile 0")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn restore_4_5_to_2025_12() {
    let (from, to) = (Dialect::V4_5, Dialect::V2025_12);
    let run = restore("b", from, board(to));
    let r = check_ok(&run, from, to);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    let sem = skipped(&r, R::SemanticsChanged);
    assert!(sem.contains(&"set d_roll = 35") && sem.contains(&"set d_min_roll = 25"));
    assert!(r.warnings.iter().any(|w| w.contains("Re-tune D")));
    assert!(skipped(&r, R::RemovedOnTarget).contains(&"feature RX_MSP"));
    assert!(skipped(&r, R::RemovedOnTarget).contains(&"beeper -RC_SMOOTHING_INIT_FAIL"));
    assert!(skipped(&r, R::RemovedOnTarget).contains(&"set iterm_limit = 300"));
    assert!(skipped(&r, R::NotInBuild).contains(&"set expresslrs_uid = ***"));
    run.src.fc(|fc| {
        let s = fc.snapshot();
        assert_eq!(s.setting("motor_idle"), Some("450"));
        assert_eq!(s.setting("debug_mode"), Some("D_MAX"));
        assert_eq!(s.setting("angle_p_gain"), Some("60"));
        for l in [
            "set motor_idle = 450",
            "beeper -CRASHFLIP",
            "serial UART1 2 115200 57600 0 115200",
            "serial UART3 131073 115200 57600 0 115200",
            "adjrange 1 0 3 900 1300 25 3 0 0",
        ] {
            assert!(fc.log.iter().any(|x| x == l), "missing `{l}`");
        }
        assert!(!fc.log.iter().any(|x| x.starts_with("resource")));
    });
}

#[test]
fn restore_4_4_to_4_5_renames_angle_settings() {
    let (from, to) = (Dialect::V4_4, Dialect::V4_5);
    let run = restore("c", from, board(to));
    let r = check_ok(&run, from, to);
    assert!(skipped(&r, R::SemanticsChanged).contains(&"set horizon_transition = 70"));
    assert!(skipped(&r, R::RemovedOnTarget).contains(&"set acc_hardware = FAKE"));
    run.src.fc(|fc| {
        let s = fc.snapshot();
        assert_eq!(s.setting("angle_p_gain"), Some("60"));
        assert_eq!(s.setting("angle_limit"), Some("60"));
        assert_eq!(s.setting("d_roll"), Some("35"), "4.4 -> 4.5 keeps D");
        assert_eq!(s.setting("dshot_idle_value"), Some("450"));
    });
}

#[test]
fn restore_2026_6_to_2025_12_downgrade() {
    let (from, to) = (Dialect::V2026_6, Dialect::V2025_12);
    let run = restore("d", from, board(to));
    let r = check_ok(&run, from, to);
    assert!(r.failed.is_empty(), "{:?}", r.failed);
    // Selected battery profile 1 lands in master; profile 0 is skipped.
    let other = skipped(&r, R::OtherBatteryProfile);
    assert!(other.contains(&"set bat_capacity = 1500"), "{other:?}");
    let removed = skipped(&r, R::RemovedOnTarget);
    assert!(removed.contains(&"battery_profile 1"));
    assert!(removed.contains(&"set battery_profile_name = 4S850"));
    assert!(removed.contains(&"waypoint clear"));
    let none = skipped(&r, R::NoEquivalentMode);
    assert!(none.contains(&"aux 6 56 4 1700 2100 0 0"));
    assert!(
        none.contains(&"aux 7 13 4 900 1300 0 56"),
        "linkedTo AUTOPILOT"
    );
    assert!(
        none.contains(&"adjrange 1 0 3 900 1300 33 3 0 0"),
        "BATTERY_PROFILE"
    );
    assert!(
        none.contains(&"adjrange 2 0 3 1300 1700 32 3 0 0"),
        "SIMPLIFIED_MASTER_MULTIPLIER"
    );
    assert!(r
        .warnings
        .iter()
        .any(|w| w.contains("Only battery profile 1")));
    assert!(r.warnings.iter().any(|w| w.contains("UART4 (mask 524288)")));
    run.src.fc(|fc| {
        let s = fc.snapshot();
        assert_eq!(s.setting("bat_capacity"), Some("850"));
        assert_eq!(s.setting("vbat_max_cell_voltage"), Some("440"));
        assert_eq!(s.setting("mag_hardware"), Some("QMC5883"));
        for l in [
            "adjrange 0 0 3 1700 2100 29 3 0 0",
            "serial UART4 1 115200 57600 0 115200",
        ] {
            assert!(fc.log.iter().any(|x| x == l), "missing `{l}`");
        }
    });
}

#[test]
fn restore_2026_6_to_4_5_and_4_4_to_2026_6() {
    for (from, to) in [
        (Dialect::V2026_6, Dialect::V4_5),
        (Dialect::V4_4, Dialect::V2026_6),
    ] {
        let run = restore("e", from, board(to));
        let r = check_ok(&run, from, to);
        assert!(r.failed.is_empty(), "{from:?}->{to:?}: {:?}", r.failed);
    }
}

#[test]
fn restore_same_line_replays_hardware_and_keeps_profiles() {
    for d in Dialect::ALL {
        let run = restore("f", d, board(d));
        let r = check_ok(&run, d, d);
        assert!(r.failed.is_empty(), "{d:?}: {:?}", r.failed);
        assert!(skipped(&r, R::HardwareLineChanged).is_empty());
        run.src.fc(|fc| {
            for l in [
                "resource LED_STRIP 1 A08",
                "timer A08 AF1",
                "dma pin A08 0",
                "profile 1",
                "rateprofile 0",
            ] {
                assert!(fc.log.iter().any(|x| x == l), "{d:?}: missing `{l}`");
            }
            assert_eq!(
                fc.snapshot().setting("p_roll"),
                Some("55"),
                "flat mock: last profile wins"
            );
        });
    }
}

/// Steering (a): after a full mass erase the board is at defaults; same restore, UID still
/// checked, `defaults nosave` harmless. Without the erase, leftovers are reset too.
#[test]
fn restore_after_full_erase_and_over_leftover_settings() {
    let (from, to) = (Dialect::V2025_12, Dialect::V2026_6);
    let erased = restore("g", from, board(to));
    let a = check_ok(&erased, from, to);
    let mut used = board(to);
    used.preset(&["set craft_name = OLD", "set small_angle = 180"]);
    let kept = restore("h", from, used);
    let b = check_ok(&kept, from, to);
    assert_eq!(a.applied, b.applied);
    kept.src.fc(|fc| {
        let s = fc.snapshot();
        assert_eq!(s.setting("craft_name"), Some("RESTORE"));
        assert_eq!(
            s.setting("small_angle"),
            Some("25"),
            "defaults nosave reset it"
        );
    });
}

#[test]
fn failed_lines_are_reported_and_the_rest_is_saved() {
    let (from, to) = (Dialect::V4_5, Dialect::V2025_12);
    let mut fc = board(to);
    fc.fail_on = vec!["set angle_limit".into(), "set expresslrs_uid".into()];
    let run = restore("i", from, fc);
    let r = check_ok(&run, from, to);
    assert_eq!(r.failed.len(), 2, "both profiles: {:?}", r.failed);
    assert_eq!(r.failed[0].line, "set angle_limit = 60");
    assert!(
        r.failed[0].error.contains("###ERROR IN set"),
        "{}",
        r.failed[0].error
    );
    run.src.fc(|fc| {
        assert_eq!(fc.saves_after_error, 1);
        assert_eq!(fc.snapshot().setting("motor_idle"), Some("450"), "saved");
    });
}

#[test]
fn failure_errors_mask_secrets_and_name_the_mapped_command() {
    let body = backup_text(Dialect::V4_5);
    let body = body.replace(
        "set craft_name = RESTORE",
        "set craft_name = RESTORE\nset dshot_idle_value = 9999",
    );
    let mut fc = board(Dialect::V2025_12);
    fc.fail_on = vec!["set motor_idle = 9999".into()];
    let run = run_with("j", &body, UID, "SPEEDYBEEF405V4", "4.5.5", Some(fc));
    let r = run.result.unwrap();
    let f = r.failed.iter().find(|f| f.line.contains("9999")).unwrap();
    assert!(
        f.error.contains("(sent as `set motor_idle = 9999`)"),
        "{}",
        f.error
    );
}

// ------------------------------------------------------------------ refusals

#[test]
fn refuses_another_board() {
    let run = run_with(
        "k",
        &backup_text(Dialect::V4_5),
        "0000000000000001000000FF",
        "SPEEDYBEEF405V4",
        "4.5.5",
        Some(board(Dialect::V4_5)),
    );
    let (reason, d) = reason_of(run.result.as_ref().unwrap_err());
    assert_eq!(reason, "BOARD_UID_MISMATCH");
    assert!(d["backupId"]
        .as_str()
        .unwrap()
        .starts_with("0000000000000001000000FF/"));
    run.src.fc(|fc| {
        assert!(!fc
            .log
            .iter()
            .any(|l| l == "batch start" || l == "defaults nosave"));
        assert_eq!(fc.reboots, 0);
    });
}

#[test]
fn unknown_uid_matches_by_board_name_only() {
    // Backup without a UID, same board name: allowed with a warning.
    let run = run_with(
        "l",
        &backup_text(Dialect::V4_5),
        "unknown",
        "speedybeef405v4",
        "4.5.5",
        Some(board(Dialect::V4_5)),
    );
    let r = check_ok(&run, Dialect::V4_5, Dialect::V4_5);
    assert!(r
        .warnings
        .iter()
        .any(|w| w.contains("matched by board name")));
    // FC without a UID: same rule.
    let mut fc = board(Dialect::V4_5);
    fc.mcu_id.clear();
    let run = run_with(
        "m",
        &backup_text(Dialect::V4_5),
        UID,
        "SPEEDYBEEF405V4",
        "4.5.5",
        Some(fc),
    );
    assert!(run
        .result
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.contains("matched by board name")));
    // Unknown UID and a different board name: refused.
    let run = run_with(
        "n",
        &backup_text(Dialect::V4_5),
        "unknown",
        "MATEKF405",
        "4.5.5",
        Some(board(Dialect::V4_5)),
    );
    assert_eq!(
        reason_of(run.result.as_ref().unwrap_err()).0,
        "BOARD_UID_MISMATCH"
    );
}

#[test]
fn refuses_a_plain_diff_and_unreadable_backups() {
    let plain = backup_text(Dialect::V4_5).replace("defaults nosave\n", "");
    let run = run_with(
        "o",
        &plain,
        UID,
        "SPEEDYBEEF405V4",
        "4.5.5",
        Some(board(Dialect::V4_5)),
    );
    let (reason, d) = reason_of(run.result.as_ref().unwrap_err());
    assert_eq!(reason, "BACKUP_NOT_DIFF_ALL");
    assert_eq!(d["path"].as_str(), Some(run.path.as_str()));
    run.src.fc(|fc| assert!(fc.log.is_empty(), "nothing sent"));

    let run = run_with(
        "p",
        &backup_text(Dialect::V4_5),
        UID,
        "SPEEDYBEEF405V4",
        "4.3.2",
        Some(board(Dialect::V4_5)),
    );
    assert_eq!(
        reason_of(run.result.as_ref().unwrap_err()).0,
        "BACKUP_UNREADABLE"
    );

    let st = store("q");
    let src = Source::new(board(Dialect::V4_5));
    let e = restore_after_flash(&src, &st, "nope/20260101T000000Z", &mut |_| {}).unwrap_err();
    let (reason, d) = reason_of(&e);
    assert_eq!(reason, "BACKUP_UNREADABLE");
    assert_eq!(d["backupId"], "nope/20260101T000000Z");
    assert!(d.get("path").is_none());
}

#[test]
fn unsupported_target_firmware_stays_unsupported_firmware() {
    let dump = fixture("4.5", "speedybeef405v4").replace("4.5.5", "4.3.2");
    let run = run_with(
        "r",
        &backup_text(Dialect::V4_5),
        UID,
        "SPEEDYBEEF405V4",
        "4.5.5",
        Some(MockFc::from_dump(&dump)),
    );
    assert_eq!(run.result.unwrap_err().code(), "UNSUPPORTED_FIRMWARE");
}

#[test]
fn not_connected_is_board_not_reconnected_without_stage() {
    let run = run_with(
        "s",
        &backup_text(Dialect::V4_5),
        UID,
        "SPEEDYBEEF405V4",
        "4.5.5",
        None,
    );
    let (reason, d) = reason_of(run.result.as_ref().unwrap_err());
    assert_eq!(reason, "BOARD_NOT_RECONNECTED");
    assert!(d.get("stage").is_none());
}

// ------------------------------------------------------------------ link failures

#[test]
fn board_dropping_mid_restore_is_board_not_reconnected_restoring() {
    let mut fc = board(Dialect::V2025_12);
    fc.link_fault = Some(("set p_roll".into(), LinkFault::Unplug));
    let run = restore("t", Dialect::V4_5, fc);
    let (reason, d) = reason_of(run.result.as_ref().unwrap_err());
    assert_eq!(reason, "BOARD_NOT_RECONNECTED");
    assert_eq!(d["stage"], "RESTORING");
    run.src.fc(|fc| {
        assert!(!fc.log.iter().any(|l| l == "save"));
        assert_eq!(
            fc.snapshot().setting("motor_idle"),
            Some("550"),
            "nothing saved"
        );
    });

    let mut fc = board(Dialect::V2025_12);
    fc.link_fault = Some(("set p_roll".into(), LinkFault::Timeout));
    let run = restore("u", Dialect::V4_5, fc);
    let (_, d) = reason_of(run.result.as_ref().unwrap_err());
    assert_eq!(d["stage"], "RESTORING");
    run.src.fc(|fc| {
        assert!(!fc.log.iter().any(|l| l == "save"), "discarded, not saved");
    });
}

#[test]
fn save_and_reboot_failures_carry_their_stage() {
    let mut fc = board(Dialect::V2026_6);
    fc.save_fault = true;
    let run = restore("v", Dialect::V2025_12, fc);
    let (reason, d) = reason_of(run.result.as_ref().unwrap_err());
    assert_eq!(
        (reason.as_str(), &d["stage"]),
        ("BOARD_NOT_RECONNECTED", &serde_json::json!("SAVING"))
    );

    let mut fc = board(Dialect::V2026_6);
    fc.reconnect_fault = true;
    let run = restore("w", Dialect::V2025_12, fc);
    let (reason, d) = reason_of(run.result.as_ref().unwrap_err());
    assert_eq!(
        (reason.as_str(), &d["stage"]),
        ("BOARD_NOT_RECONNECTED", &serde_json::json!("REBOOTING"))
    );
}

/// An FC that answers `set small_angle = ...` without storing it.
struct Forgetful(MockFc);

impl crate::device::Connection for Forgetful {
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>> {
        self.0.msp_request(code, payload)
    }
    fn enter_cli(&mut self) -> AppResult<()> {
        self.0.enter_cli()
    }
    fn cli_exec(&mut self, line: &str) -> AppResult<String> {
        if line.starts_with("set small_angle") {
            return Ok(format!("{line}\r\nsmall_angle set to 180\r\n"));
        }
        self.0.cli_exec(line)
    }
    fn exit_cli(&mut self, save: bool) -> AppResult<()> {
        self.0.exit_cli(save)
    }
    fn uid(&self) -> Option<String> {
        self.0.uid()
    }
}

impl crate::device::Reconnect for Forgetful {
    fn reconnect(&mut self, t: Duration) -> AppResult<()> {
        self.0.reconnect(t)
    }
}

struct ForgetfulSource(Mutex<Forgetful>);

impl LinkSource for ForgetfulSource {
    fn with_link<T>(&self, f: impl FnOnce(&mut dyn FcLink) -> AppResult<T>) -> AppResult<T> {
        f(&mut *self.0.lock().unwrap())
    }
}

#[test]
fn read_back_differences_are_warnings() {
    let body = backup_text(Dialect::V2025_12).replace(
        "set craft_name = RESTORE",
        "set craft_name = RESTORE\nset small_angle = 180",
    );
    let st = store("x");
    let bk = st
        .save(&body, UID, "SPEEDYBEEF405V4", "2025.12.5", UNIX_EPOCH)
        .unwrap();
    let src = ForgetfulSource(Mutex::new(Forgetful(board(Dialect::V2026_6))));
    let r = restore_after_flash(&src, &st, &bk.id, &mut |_| {}).unwrap();
    let _ = std::fs::remove_dir_all(st.root());
    assert!(r.saved);
    let rb: Vec<&String> = r
        .warnings
        .iter()
        .filter(|w| w.starts_with("Read back") && !FLAT_MOCK_READBACK.contains(&w.as_str()))
        .collect();
    assert_eq!(rb.len(), 1, "{:?}", r.warnings);
    assert_eq!(rb[0], "Read back: small_angle is 25, restored 180");
}
