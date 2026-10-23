//! Tauri commands for workstream `config`. Signatures are a contract with the UI
//! (src/api/commands.ts, docs/ARCHITECTURE.md §3 and §5.1).

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use super::backup::BackupStore;
use crate::error::{AppError, AppResult};
use crate::model::{ApplyResult, Backup, CliScript, DroneModel, FcSerialPort, VerifyReport};

// The FC-facing commands below are implemented in `super::session` (read_ports, plan,
// backup, apply, verify, rollback) against `device::Connection`. They stay
// NOT_IMPLEMENTED here until the device workstream exposes the active connection in
// Tauri state; wiring them is then a one-liner each.

/// `<app_data_dir>/backups`
fn backups_root<R: Runtime>(app: &AppHandle<R>) -> AppResult<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Io(format!("app data dir: {e}")))?;
    Ok(base.join("backups"))
}

/// Read the FC's serial ports (names, firmware identifiers, current functions).
#[tauri::command]
pub async fn config_read_ports() -> AppResult<Vec<FcSerialPort>> {
    Err(AppError::NotImplemented("config_read_ports"))
}

/// Save `diff all` from the connected FC to the backups folder.
#[tauri::command]
pub async fn config_backup() -> AppResult<Backup> {
    Err(AppError::NotImplemented("config_backup"))
}

/// Pure generation (no FC I/O): DroneModel + ports -> CLI script for preview.
#[tauri::command]
pub async fn config_generate(model: DroneModel, ports: Vec<FcSerialPort>) -> AppResult<CliScript> {
    super::generate(&model, &ports)
}

/// Read-only plan against the connected FC: reads `version`, `serial`, `dump all`
/// (firmware gate, SPI ELRS detection, aux slots, available settings) and returns the
/// script plus `cleared` ports. Writes nothing.
#[tauri::command]
pub async fn config_plan(model: DroneModel) -> AppResult<CliScript> {
    let _ = model;
    Err(AppError::NotImplemented("config_plan"))
}

/// Apply: `diff all` backup -> `batch start` + commands + `save` (batch still active) ->
/// reboot -> reconnect. Resolves only after reconnect; emits `config://progress`.
#[tauri::command]
pub async fn config_apply(script: CliScript) -> AppResult<ApplyResult> {
    let _ = script;
    Err(AppError::NotImplemented("config_apply"))
}

/// After reconnect, read back settings and compare to `model`.
#[tauri::command]
pub async fn config_verify(model: DroneModel) -> AppResult<VerifyReport> {
    let _ = model;
    Err(AppError::NotImplemented("config_verify"))
}

/// Restore a backup: replay its `diff all` (`defaults nosave` + lines) + `save` + reconnect.
#[tauri::command]
pub async fn config_rollback(backup: Backup) -> AppResult<ApplyResult> {
    let _ = backup;
    Err(AppError::NotImplemented("config_rollback"))
}

/// All backups on disk, newest first (all boards).
#[tauri::command]
pub async fn config_list_backups<R: Runtime>(app: AppHandle<R>) -> AppResult<Vec<Backup>> {
    BackupStore::new(backups_root(&app)?).list()
}

/// Absolute path of the backups root folder (UI opens it with the opener plugin).
#[tauri::command]
pub async fn config_backups_dir<R: Runtime>(app: AppHandle<R>) -> AppResult<String> {
    let root = backups_root(&app)?;
    std::fs::create_dir_all(&root)?;
    Ok(root.to_string_lossy().into_owned())
}

/// Restore by id (from config_list_backups): safety backup, then the rollback path.
#[tauri::command]
pub async fn config_restore(backup_id: String) -> AppResult<ApplyResult> {
    let _ = backup_id;
    Err(AppError::NotImplemented("config_restore"))
}
