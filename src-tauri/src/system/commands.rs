//! Tauri commands for workstream `packaging`.

use crate::error::{AppError, AppResult};
use crate::model::{AppInfo, DriverStatus};

#[tauri::command]
pub async fn system_driver_status() -> AppResult<DriverStatus> {
    Err(AppError::NotImplemented("system_driver_status"))
}

/// Bind WinUSB to the DFU device (elevated helper using libwdi). Windows only.
#[tauri::command]
pub async fn system_install_dfu_driver() -> AppResult<()> {
    Err(AppError::NotImplemented("system_install_dfu_driver"))
}

#[tauri::command]
pub fn system_app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        cache_dir: String::new(),
    }
}
