//! Tauri commands for workstream `device`. Signatures are a contract with the UI
//! (src/api/commands.ts) - do not change without a `shared` PR.

use crate::error::{AppError, AppResult};
use crate::model::{FcInfo, HostPort};

/// List host serial ports, classified by VID/PID.
#[tauri::command]
pub async fn device_list_ports() -> AppResult<Vec<HostPort>> {
    Err(AppError::NotImplemented("device_list_ports"))
}

/// Open `path` (e.g. "COM5") at 115200 8N1 and keep it as the active connection.
#[tauri::command]
pub async fn device_connect(path: String) -> AppResult<()> {
    let _ = path;
    Err(AppError::NotImplemented("device_connect"))
}

#[tauri::command]
pub async fn device_disconnect() -> AppResult<()> {
    Err(AppError::NotImplemented("device_disconnect"))
}

/// Read FC identity via MSP (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO, UID, BUILD_INFO).
#[tauri::command]
pub async fn device_identify() -> AppResult<FcInfo> {
    Err(AppError::NotImplemented("device_identify"))
}

/// Run one raw CLI line (enters CLI mode if needed). For diagnostics/advanced use.
#[tauri::command]
pub async fn device_cli_exec(line: String) -> AppResult<String> {
    let _ = line;
    Err(AppError::NotImplemented("device_cli_exec"))
}

/// Reboot the connected FC into its ROM DFU bootloader (MSP_SET_REBOOT / CLI `bl`).
#[tauri::command]
pub async fn device_reboot_to_bootloader() -> AppResult<()> {
    Err(AppError::NotImplemented("device_reboot_to_bootloader"))
}
