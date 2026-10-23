//! Tauri commands for workstream `config`.

use crate::error::{AppError, AppResult};
use crate::model::{ApplyResult, Backup, CliScript, DroneModel, FcSerialPort, VerifyReport};

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

/// Apply: `batch start` + commands + `batch end` + `save`. Requires a prior backup.
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

/// Restore a backup: `defaults nosave` + backup lines + `save`.
#[tauri::command]
pub async fn config_rollback(backup: Backup) -> AppResult<ApplyResult> {
    let _ = backup;
    Err(AppError::NotImplemented("config_rollback"))
}
