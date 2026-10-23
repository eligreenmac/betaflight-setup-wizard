// SPDX-License-Identifier: GPL-3.0-or-later
//! Tauri commands for workstream `flasher`. Everything blocking (HTTP, USB) runs on
//! `spawn_blocking`; progress goes out on `flasher://progress`.

use crate::device::ConnectionManager;
use crate::error::{AppError, AppResult};
use crate::flasher::build_api::{BuildApi, ReqwestClient};
use crate::flasher::dfu::DfuSe;
use crate::flasher::error::FlashError;
use crate::flasher::service::{self, FirmwareService};
use crate::flasher::{usb, PROGRESS_EVENT};
use crate::model::{
    BoardTarget, DfuDevice, FirmwareImage, FirmwareRelease, FirmwareRequest, FirmwareRequirements,
    FlashOptions, FlashProgress, FlashStage,
};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// One flasher USB operation at a time; a second caller gets `PORT_BUSY {heldBy: "APP"}`.
static DFU_LOCK: Mutex<()> = Mutex::new(());

async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, FlashError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::FlashFailed(format!("worker failed: {e}")))?
        .map_err(AppError::from)
}

fn firmware_service<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<FirmwareService<ReqwestClient>, FlashError> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| FlashError::Io(std::io::Error::other(e.to_string())))?
        .join("firmware");
    FirmwareService::new(BuildApi::with_reqwest(env!("CARGO_PKG_VERSION"))?, root)
}

fn emit<R: Runtime>(app: &AppHandle<R>, p: FlashProgress) {
    if let Err(e) = app.emit(PROGRESS_EVENT, &p) {
        log::warn!("cannot emit {PROGRESS_EVENT}: {e}");
    }
}

fn emit_failed<R: Runtime, T>(
    app: &AppHandle<R>,
    r: Result<T, FlashError>,
) -> Result<T, FlashError> {
    if let Err(e) = &r {
        emit(
            app,
            FlashProgress {
                stage: FlashStage::Failed,
                fraction: 0.0,
                message: Some(e.to_string()),
            },
        );
    }
    r
}

fn dfu_guard(what: &str) -> Result<std::sync::MutexGuard<'static, ()>, FlashError> {
    DFU_LOCK
        .try_lock()
        .map_err(|_| FlashError::Busy(what.to_string()))
}

/// All targets (Build API `/api/targets`, cached for offline use).
#[tauri::command]
pub async fn flasher_list_targets(app: AppHandle) -> AppResult<Vec<BoardTarget>> {
    blocking(move || firmware_service(&app)?.list_targets()).await
}

/// Releases available for a target: withdrawn releases and releases outside the supported
/// lines (`config::firmware::is_supported`, e.g. 2026.12.0-alpha) are hidden.
#[tauri::command]
pub async fn flasher_list_releases(
    app: AppHandle,
    target: String,
) -> AppResult<Vec<FirmwareRelease>> {
    blocking(move || firmware_service(&app)?.list_releases(&target)).await
}

/// USB devices currently in DFU mode. `driverOk: false` = Windows has no WinUSB bound.
#[tauri::command]
pub async fn flasher_list_dfu_devices() -> AppResult<Vec<DfuDevice>> {
    blocking(|| Ok(usb::list_dfu()?.into_iter().map(|f| f.info).collect())).await
}

/// Return a firmware image from cache, or request a cloud build and cache it. The board's
/// options are filtered to what the release offers (`droppedOptions`), required options are
/// added (`REQUIRED_OPTION_UNSUPPORTED`), and no build is requested without the release's
/// option list (`BUILD_OPTIONS_UNAVAILABLE`). Emits `flasher://progress` with stage `DOWNLOADING`.
#[tauri::command]
pub async fn flasher_fetch_firmware(
    app: AppHandle,
    request: FirmwareRequest,
) -> AppResult<FirmwareImage> {
    blocking(move || {
        let svc = firmware_service(&app)?;
        let r = svc.fetch(&request, &mut |p| emit(&app, p), &AtomicBool::new(false));
        emit_failed(&app, r)
    })
    .await
}

/// Build API option values the wizard's answers need (`FirmwareRequest.requiredOptions`):
/// receiver protocol and OSD type (ARCHITECTURE §6 "Build options"). With `release`, also
/// checks that release offers them (`REQUIRED_OPTION_UNSUPPORTED`; the option list may be
/// fetched: `BUILD_OPTIONS_UNAVAILABLE`). Read-only; no FC or USB I/O.
#[tauri::command]
pub async fn flasher_required_options(
    app: AppHandle,
    requirements: FirmwareRequirements,
    release: Option<String>,
) -> AppResult<Vec<String>> {
    blocking(move || firmware_service(&app)?.required_options(&requirements, release.as_deref()))
        .await
}

/// Cached firmware images available offline.
#[tauri::command]
pub async fn flasher_list_cache(app: AppHandle) -> AppResult<Vec<FirmwareImage>> {
    blocking(move || firmware_service(&app)?.list_cache()).await
}

/// Flash `image` to the DFU device with serial `dfu_serial` (or the only one present).
/// Emits `flasher://progress`. Fails with `READ_PROTECTED` before erasing anything.
#[tauri::command]
pub async fn flasher_flash(
    app: AppHandle,
    image: FirmwareImage,
    dfu_serial: Option<String>,
    options: FlashOptions,
) -> AppResult<()> {
    blocking(move || {
        let r = (|| {
            let _guard = dfu_guard("DFU device")?;
            // Only files inside the firmware cache (canonicalized; no symlink escapes).
            let hex = firmware_service(&app)?.load_image(&image)?;
            let found = usb::select(usb::list_dfu()?, dfu_serial.as_deref())?;
            let mut dfu = DfuSe::new(usb::open_selected(found)?);
            service::flash(
                &mut dfu,
                &hex,
                &options,
                &mut |p| emit(&app, p),
                &AtomicBool::new(false),
            )
        })();
        if r.is_ok() {
            // RAM changes of a board rebooted into DFU are gone now (§4.1 "RAM state").
            app.state::<ConnectionManager>()
                .forget_ram_state_after_flash();
        }
        emit_failed(&app, r)
    })
    .await
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
    if !confirm_erase {
        return Err(AppError::EraseConfirmationRequired);
    }
    blocking(move || {
        let _guard = dfu_guard("DFU device")?;
        let found = usb::select(usb::list_dfu()?, dfu_serial.as_deref())?;
        DfuSe::new(usb::open_selected(found)?).remove_read_protection(true)
    })
    .await
}
