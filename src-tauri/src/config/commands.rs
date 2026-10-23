//! Tauri commands for workstream `config`. Signatures are a contract with the UI
//! (src/api/commands.ts, docs/ARCHITECTURE.md §3 and §5.1).

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

/// PREVIEW ONLY. Pure generation (no FC I/O): DroneModel + caller-supplied ports -> CLI
/// script. It assumes default baud rates and free aux slots and cannot check SPI receiver
/// hardware, so its output must never be applied: `config_apply` takes the script from
/// `config_plan`.
#[tauri::command]
pub async fn config_generate(model: DroneModel, ports: Vec<FcSerialPort>) -> AppResult<CliScript> {
    super::generate(&model, &ports)
}

/// Binding phrase -> 6-byte ExpressLRS UID (first 6 bytes of
/// MD5(`-DMY_BINDING_PHRASE="<phrase>"`)). Pure. The phrase is never stored or logged;
/// the UI puts only the returned UID in `receiver.elrsUid`.
#[tauri::command]
pub async fn config_elrs_uid(phrase: String) -> AppResult<[u8; 6]> {
    let _ = phrase;
    Err(AppError::NotImplemented("config_elrs_uid"))
}

/// Read-only plan against the connected FC: reads `version`, `serial`, `dump all`
/// (firmware gate, SPI ELRS detection, aux slots, available settings) and returns the
/// script plus `cleared` ports. Writes nothing. This is the script the UI applies.
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
pub async fn config_list_backups() -> AppResult<Vec<Backup>> {
    Err(AppError::NotImplemented("config_list_backups"))
}

/// Absolute path of the backups root folder (UI opens it with the opener plugin).
#[tauri::command]
pub async fn config_backups_dir() -> AppResult<String> {
    Err(AppError::NotImplemented("config_backups_dir"))
}

/// Restore by id (from config_list_backups): safety backup, then the rollback path.
#[tauri::command]
pub async fn config_restore(backup_id: String) -> AppResult<ApplyResult> {
    let _ = backup_id;
    Err(AppError::NotImplemented("config_restore"))
}
