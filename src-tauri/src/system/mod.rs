//! Workstream `packaging`: Windows driver checks/installation (WinUSB via
//! libwdi for DFU, vendor guidance for CP210x/CH34x), app/environment info.
//! Installer config lives in tauri.conf.json and .github/workflows; see
//! docs/PACKAGING.md.

pub mod commands;
pub mod usb_drivers;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
use crate::error::AppError;
use crate::error::AppResult;
use crate::model::DriverStatus;
use std::path::PathBuf;

/// File name of the bundled libwdi `wdi-simple` build (see docs/PACKAGING.md).
pub const HELPER_EXE: &str = "bfsw-wdi-simple.exe";

/// Where the helper lives inside the installed app (`<resources>/drivers/`).
pub fn helper_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path()
        .resource_dir()
        .ok()
        .map(|d| d.join("drivers").join(HELPER_EXE))
}

#[cfg(windows)]
pub fn driver_status() -> AppResult<DriverStatus> {
    Ok(usb_drivers::evaluate(&windows::list_usb_devices()?))
}

#[cfg(not(windows))]
pub fn driver_status() -> AppResult<DriverStatus> {
    let mut s = usb_drivers::evaluate(&[]);
    s.notes
        .push("USB driver checks only apply on Windows".to_string());
    Ok(s)
}

#[cfg(not(windows))]
pub fn install_dfu_driver(_helper: Option<PathBuf>) -> AppResult<()> {
    Ok(())
}

/// Detect first; only devices without WinUSB trigger the elevated helper.
#[cfg(windows)]
pub fn install_dfu_driver(helper: Option<PathBuf>) -> AppResult<()> {
    let before = driver_status()?;
    let todo = usb_drivers::dfu_ids_needing_install(&before);
    if todo.is_empty() {
        return Ok(());
    }
    let helper = helper.filter(|p| p.is_file()).ok_or_else(|| {
        AppError::Driver(format!(
            "WinUSB helper ({HELPER_EXE}) is not bundled with this build"
        ))
    })?;
    for (vid, pid, name) in &todo {
        let dest = std::env::temp_dir()
            .join("bfsw-winusb")
            .join(format!("{vid:04x}_{pid:04x}"));
        std::fs::create_dir_all(&dest)?;
        let args = usb_drivers::wdi_simple_args(*vid, *pid, name, &dest.to_string_lossy())
            .iter()
            .map(|a| usb_drivers::quote_windows_arg(a))
            .collect::<Vec<_>>()
            .join(" ");
        let code = windows::run_elevated(&helper, &args, &dest, 5 * 60 * 1000)?;
        if code != 0 {
            return Err(AppError::Driver(format!(
                "WinUSB install for {name} ({vid:04X}:{pid:04X}) failed: {} (libwdi {code})",
                usb_drivers::wdi_error_message(code)
            )));
        }
    }
    // Verify: every device we touched must now be bound to WinUSB.
    let after = driver_status()?;
    let still = usb_drivers::dfu_ids_needing_install(&after);
    if let Some((vid, pid, name)) = still.first() {
        return Err(AppError::Driver(format!(
            "{name} ({vid:04X}:{pid:04X}) still has no WinUSB driver; replug the board and retry"
        )));
    }
    Ok(())
}
