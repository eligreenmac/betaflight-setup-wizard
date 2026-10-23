//! FC-facing operations over a `device::Connection`: read ports (C2), plan, backup (C4),
//! apply (C5), verify and rollback/restore (C6).
//!
//! All functions assume they may enter the CLI. Read-only operations do not leave it
//! (in Betaflight `exit` reboots the FC); the caller decides when to leave.

use std::time::SystemTime;

use super::backup::BackupStore;
use super::firmware::{parse_version, Dialect, VersionInfo};
use super::generate::generate_for;
use super::guard::{replay_lines, validate_script};
use super::modes;
use super::snapshot::{parse_dump, parse_serial, Snapshot};
use crate::device::Connection;
use crate::error::{AppError, AppResult};
use crate::model::{
    ApplyResult, Backup, CliError, CliScript, ConfigProgress, ConfigStage, DroneModel,
    FcSerialPort, Mismatch, VerifyReport,
};

/// Waits for the FC to come back after a reboot (`save` / `exit`) and re-opens the link.
/// Implemented by the device workstream's connection; by the mock in tests.
pub trait Reconnect {
    fn reconnect(&mut self) -> AppResult<()>;
}

pub type Progress<'a> = &'a mut dyn FnMut(ConfigProgress);

fn progress(cb: &mut dyn FnMut(ConfigProgress), stage: ConfigStage, fraction: f32, msg: &str) {
    cb(ConfigProgress {
        stage,
        fraction,
        message: (!msg.is_empty()).then(|| msg.to_string()),
    });
}

/// Did the FC reject `command`? Betaflight prints `###ERROR IN <cmd>: <reason>###`;
/// enabling a feature compiled out prints "... unavailable".
pub fn is_cli_error(command: &str, response: &str) -> bool {
    if response.contains("###ERROR") {
        return true;
    }
    let enabling_feature = command.starts_with("feature ") && !command.starts_with("feature -");
    enabling_feature && response.to_ascii_lowercase().contains("unavailable")
}

/// `version` -> parsed info + supported dialect (UNSUPPORTED_FIRMWARE otherwise).
pub fn read_version<C: Connection + ?Sized>(conn: &mut C) -> AppResult<(VersionInfo, Dialect)> {
    conn.enter_cli()?;
    let out = conn.cli_exec("version")?;
    let info = parse_version(&out)
        .ok_or_else(|| AppError::Cli(format!("unexpected `version` output: {}", out.trim())))?;
    let dialect = Dialect::from_version(&info.version)?;
    Ok((info, dialect))
}

/// C2: `serial` -> `FcSerialPort[]` with dialect-specific identifiers.
pub fn read_ports<C: Connection + ?Sized>(conn: &mut C) -> AppResult<Vec<FcSerialPort>> {
    let (_, dialect) = read_version(conn)?;
    let out = conn.cli_exec("serial")?;
    Ok(parse_serial(&out, dialect)
        .iter()
        .map(|p| p.to_fc_port())
        .collect())
}

/// `dump all` -> snapshot (gated on supported firmware).
pub fn read_snapshot<C: Connection + ?Sized>(conn: &mut C) -> AppResult<(Snapshot, Dialect)> {
    let (info, dialect) = read_version(conn)?;
    let mut snap = parse_dump(&conn.cli_exec("dump all")?);
    if snap.version.is_none() {
        snap.version = Some(info.version);
    }
    if snap.board_name.is_none() {
        snap.board_name = info.board_name;
    }
    Ok((snap, dialect))
}

/// Read-only plan against the connected FC (config_plan).
pub fn plan<C: Connection + ?Sized>(conn: &mut C, model: &DroneModel) -> AppResult<CliScript> {
    let (snap, dialect) = read_snapshot(conn)?;
    let mut script = generate_for(model, dialect, &snap.ports, Some(&snap))?;
    if !model.fc.firmware_version.is_empty()
        && Dialect::from_version(&model.fc.firmware_version).ok() != Some(dialect)
    {
        script.warnings.push(format!(
            "the model was made for Betaflight {}, the FC runs {}; generated for {}",
            model.fc.firmware_version,
            snap.version.as_deref().unwrap_or("?"),
            dialect.as_str()
        ));
    }
    Ok(script)
}

/// C4: `diff all` -> backup file.
pub fn backup<C: Connection + ?Sized>(
    conn: &mut C,
    store: &BackupStore,
    board_uid: &str,
) -> AppResult<Backup> {
    let (info, _) = read_version(conn)?;
    let diff = conn.cli_exec("diff all")?;
    if diff.contains("###ERROR") || diff.trim().is_empty() {
        return Err(AppError::Cli(
            "`diff all` failed; nothing was changed".into(),
        ));
    }
    let board = info
        .board_name
        .or_else(|| parse_dump(&diff).board_name)
        .unwrap_or_else(|| "unknown".into());
    store.save(&diff, board_uid, &board, &info.version, SystemTime::now())
}

/// Send `lines` inside a CLI batch, stop at the first rejected command. Returns
/// (applied, errors). `lines` must already be validated (`guard`); the batch is
/// opened here and closed by `finish` (`save`) or discarded (`exit`).
///
/// If the link itself fails (write/read error, timeout, disconnect) anywhere in the
/// batch, a best-effort `exit` is sent so the FC does not stay in CLI mode with a
/// half-applied, unsaved batch (`exit` reboots without saving); its own error is
/// ignored and the original error is returned. `save` is never sent on that path.
fn send_batch<C: Connection + ?Sized>(
    conn: &mut C,
    lines: &[String],
    cb: &mut dyn FnMut(ConfigProgress),
) -> AppResult<(usize, Vec<CliError>)> {
    match send_batch_inner(conn, lines, cb) {
        Ok(r) => Ok(r),
        Err(e) => {
            let _ = conn.exit_cli(false);
            Err(e)
        }
    }
}

fn send_batch_inner<C: Connection + ?Sized>(
    conn: &mut C,
    lines: &[String],
    cb: &mut dyn FnMut(ConfigProgress),
) -> AppResult<(usize, Vec<CliError>)> {
    let mut applied = 0;
    let total = lines.len().max(1) as f32;
    let r = conn.cli_exec("batch start")?;
    if is_cli_error("batch start", &r) {
        return Ok((
            0,
            vec![CliError {
                command: "batch start".into(),
                response: r.trim().to_string(),
            }],
        ));
    }
    for (i, line) in lines.iter().enumerate() {
        let resp = conn.cli_exec(line)?;
        if is_cli_error(line, &resp) {
            return Ok((
                applied,
                vec![CliError {
                    command: line.clone(),
                    response: resp.trim().to_string(),
                }],
            ));
        }
        applied += 1;
        progress(cb, ConfigStage::Sending, (i + 1) as f32 / total, line);
    }
    Ok((applied, Vec::new()))
}

/// Shared tail of apply/rollback: save (or discard on error), reboot, reconnect.
fn finish<C: Connection + Reconnect + ?Sized>(
    conn: &mut C,
    applied: usize,
    errors: Vec<CliError>,
    backup: Option<Backup>,
    cb: &mut dyn FnMut(ConfigProgress),
) -> AppResult<ApplyResult> {
    let saved = errors.is_empty();
    if saved {
        progress(cb, ConfigStage::Saving, 1.0, "save");
    }
    // Without save the unsaved batch is discarded (`exit` reboots without saving).
    conn.exit_cli(saved)?;
    progress(cb, ConfigStage::Rebooting, 1.0, "");
    conn.reconnect()?;
    progress(cb, ConfigStage::Reconnected, 1.0, "");
    Ok(ApplyResult {
        applied,
        errors,
        saved,
        backup,
        reconnected: true,
    })
}

/// C5: backup -> `batch start` + commands -> `save` (batch still active) -> reboot -> reconnect.
/// The script is validated first (`guard::validate_script`: generator verbs only, no
/// save/defaults/reboot/batch/...); a bad line is a CONFIG error and nothing is sent.
/// The first rejected command aborts the batch; nothing is saved.
pub fn apply<C: Connection + Reconnect + ?Sized>(
    conn: &mut C,
    script: &CliScript,
    store: &BackupStore,
    board_uid: &str,
    cb: Progress,
) -> AppResult<ApplyResult> {
    validate_script(&script.commands)?;
    progress(cb, ConfigStage::Backup, 0.0, "diff all");
    let (_, dialect) = read_version(conn)?;
    if Dialect::from_dialect_str(&script.dialect) != Some(dialect) {
        return Err(AppError::Config(format!(
            "script was generated for Betaflight {}, the FC runs the {} line; plan again",
            script.dialect,
            dialect.as_str()
        )));
    }
    let bk = backup(conn, store, board_uid)?;
    progress(cb, ConfigStage::Backup, 1.0, &bk.path);
    let (applied, errors) = send_batch(conn, &script.commands, cb)?;
    finish(conn, applied, errors, Some(bk), cb)
}

/// C6: read back (`dump all`) and compare with what `model` requires.
/// Regenerating against the post-apply state must be a no-op; anything it would
/// still change is a mismatch.
pub fn verify<C: Connection + ?Sized>(conn: &mut C, model: &DroneModel) -> AppResult<VerifyReport> {
    let (snap, dialect) = read_snapshot(conn)?;
    let script = generate_for(model, dialect, &snap.ports, Some(&snap))?;
    let mut mismatches = Vec::new();
    for cmd in &script.commands {
        let mut t = cmd.split_whitespace();
        match t.next() {
            Some("serial") => {
                let id = t.next().unwrap_or_default();
                let want: u32 = t.next().and_then(|m| m.parse().ok()).unwrap_or_default();
                let port = snap
                    .ports
                    .iter()
                    .find(|p| p.name == id || p.identifier.to_string() == id);
                mismatches.push(Mismatch {
                    setting: format!("serial {}", port.map(|p| p.name.as_str()).unwrap_or(id)),
                    expected: want.to_string(),
                    actual: port
                        .map(|p| p.function_mask.to_string())
                        .unwrap_or("missing".into()),
                });
            }
            Some("feature") => {
                let f = t.next().unwrap_or_default();
                let (name, want) = match f.strip_prefix('-') {
                    Some(n) => (n, false),
                    None => (f, true),
                };
                let have = snap.features.get(name).copied();
                if have != Some(want) {
                    mismatches.push(Mismatch {
                        setting: format!("feature {name}"),
                        expected: on(want),
                        actual: have.map(on).unwrap_or_else(|| "missing".into()),
                    });
                }
            }
            Some("set") => {
                if let Some((k, v)) = cmd[3..].split_once('=') {
                    let (k, v) = (k.trim(), v.trim());
                    let have = snap.setting(k);
                    if !have.is_some_and(|h| h.eq_ignore_ascii_case(v)) {
                        mismatches.push(Mismatch {
                            setting: format!("set {k}"),
                            expected: v.to_string(),
                            actual: have.unwrap_or("missing").to_string(),
                        });
                    }
                }
            }
            Some("aux") => {
                let slot: u8 = t.next().and_then(|s| s.parse().ok()).unwrap_or(255);
                let mode: u8 = t.next().and_then(|s| s.parse().ok()).unwrap_or(255);
                let have = snap.aux.iter().find(|a| a.slot == slot).map(|a| a.to_cli());
                if have.as_deref() != Some(cmd.as_str()) {
                    let label = if cmd.ends_with(" 900 900 0 0") {
                        format!("aux slot {slot}")
                    } else {
                        format!("aux {}", modes::name_of(mode).unwrap_or("?"))
                    };
                    mismatches.push(Mismatch {
                        setting: label,
                        expected: cmd.clone(),
                        actual: have.unwrap_or_else(|| "missing".into()),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(VerifyReport {
        ok: mismatches.is_empty(),
        mismatches,
    })
}

fn on(b: bool) -> String {
    if b { "ON" } else { "OFF" }.to_string()
}

/// C6: restore a backup. Takes a safety backup first, refuses a different firmware line,
/// replays the stored `diff all`, saves and reconnects. The backup is validated before
/// anything is sent (`guard::replay_lines` documents exactly which lines are replayed);
/// a tampered or foreign file is a CONFIG error.
pub fn rollback<C: Connection + Reconnect + ?Sized>(
    conn: &mut C,
    target: &Backup,
    store: &BackupStore,
    board_uid: &str,
    cb: Progress,
) -> AppResult<ApplyResult> {
    let text = std::fs::read_to_string(&target.path)?;
    let lines = replay_lines(&text)?;
    progress(cb, ConfigStage::Backup, 0.0, "diff all");
    let (info, dialect) = read_version(conn)?;
    if Dialect::from_version(&target.firmware_version).ok() != Some(dialect) {
        return Err(AppError::Config(format!(
            "backup was taken on Betaflight {}, the FC runs {}; restore is only possible on the same firmware line",
            target.firmware_version, info.version
        )));
    }
    let safety = backup(conn, store, board_uid)?;
    progress(cb, ConfigStage::Backup, 1.0, &safety.path);
    let (applied, errors) = send_batch(conn, &lines, cb)?;
    finish(conn, applied, errors, Some(safety), cb)
}
