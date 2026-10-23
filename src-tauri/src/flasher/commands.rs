//! Tauri commands for workstream `flasher`.

use crate::error::{AppError, AppResult};
use crate::model::{
    BoardTarget, DfuDevice, FirmwareImage, FirmwareRelease, FirmwareRequest, FlashOptions,
};

/// All targets (Build API `/api/targets`, cached for offline use).
#[tauri::command]
pub async fn flasher_list_targets() -> AppResult<Vec<BoardTarget>> {
    Err(AppError::NotImplemented("flasher_list_targets"))
}

/// Releases available for a target.
#[tauri::command]
pub async fn flasher_list_releases(target: String) -> AppResult<Vec<FirmwareRelease>> {
    let _ = target;
    Err(AppError::NotImplemented("flasher_list_releases"))
}

/// USB devices currently in DFU mode.
#[tauri::command]
pub async fn flasher_list_dfu_devices() -> AppResult<Vec<DfuDevice>> {
    Err(AppError::NotImplemented("flasher_list_dfu_devices"))
}

/// Return a firmware image from cache, or request a cloud build and cache it.
#[tauri::command]
pub async fn flasher_fetch_firmware(request: FirmwareRequest) -> AppResult<FirmwareImage> {
    let _ = request;
    Err(AppError::NotImplemented("flasher_fetch_firmware"))
}

/// Cached firmware images available offline.
#[tauri::command]
pub async fn flasher_list_cache() -> AppResult<Vec<FirmwareImage>> {
    Err(AppError::NotImplemented("flasher_list_cache"))
}

/// Flash `image` to the DFU device with serial `dfu_serial` (or the only one present).
/// Emits `flasher://progress`.
#[tauri::command]
pub async fn flasher_flash(
    image: FirmwareImage,
    dfu_serial: Option<String>,
    options: FlashOptions,
) -> AppResult<()> {
    let _ = (image, dfu_serial, options);
    Err(AppError::NotImplemented("flasher_flash"))
}

/// Remove STM32 read protection (option bytes). ERASES THE WHOLE CHIP; the device
/// re-enumerates afterwards. Without `confirm_erase == true` returns
/// `ERASE_CONFIRMATION_REQUIRED` and sends nothing. Never called automatically:
/// `flasher_flash` fails with `READ_PROTECTED` and the UI asks the user.
#[tauri::command]
pub async fn flasher_remove_read_protection(
    dfu_serial: Option<String>,
    confirm_erase: bool,
) -> AppResult<()> {
    let _ = (dfu_serial, confirm_erase);
    Err(AppError::NotImplemented("flasher_remove_read_protection"))
}
