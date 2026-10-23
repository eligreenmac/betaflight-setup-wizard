//! Tauri commands for workstream `packaging`.

use crate::error::{AppError, AppResult};
use crate::model::{AppInfo, DriverStatus};

/// Connected DFU / VCP / USB-UART bridge devices, their bound Windows driver,
/// and official vendor download links. Never needs elevation.
/// Non-Windows: everything OK, no devices (drivers are not an issue there).
#[tauri::command]
pub async fn system_driver_status() -> AppResult<DriverStatus> {
    tauri::async_runtime::spawn_blocking(super::driver_status)
        .await
        .map_err(|e| AppError::Driver(format!("driver status task failed: {e}")))?
}

/// Bind WinUSB to every connected DFU device that lacks it, using the bundled
/// libwdi helper (`drivers/bfsw-wdi-simple.exe`) started with a UAC prompt.
/// No-op (Ok) when nothing needs installing or on non-Windows.
#[tauri::command]
pub async fn system_install_dfu_driver(app: tauri::AppHandle) -> AppResult<()> {
    let helper = super::helper_path(&app);
    tauri::async_runtime::spawn_blocking(move || super::install_dfu_driver(helper))
        .await
        .map_err(|e| AppError::Driver(format!("driver install task failed: {e}")))?
}

#[tauri::command]
pub fn system_app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        cache_dir: String::new(),
    }
}
