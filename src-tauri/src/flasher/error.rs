// SPDX-License-Identifier: GPL-3.0-or-later
//! Internal error type of the flasher module and its mapping to the shared
//! [`AppError`] codes (ARCHITECTURE.md §2.3).

use crate::error::{AppError, PortHolder};

#[derive(Debug, thiserror::Error)]
pub enum FlashError {
    #[error("USB error: {0}")]
    Usb(String),
    /// The DFU device is present but cannot be opened (Windows: no WinUSB driver bound).
    #[error("USB driver missing for {vid:04x}:{pid:04x}: {detail}")]
    DriverMissing { vid: u16, pid: u16, detail: String },
    #[error("{0}")]
    DfuNotFound(String),
    #[error("DFU {step} failed: status {status:#04x}, state {state}")]
    Dfu { step: String, status: u8, state: u8 },
    #[error("DFU device did not return to idle after {0} attempts (state {1})")]
    DfuStuck(u32, u8),
    #[error("chip is read protected; removing protection erases the whole chip")]
    ReadProtected,
    #[error("removing read protection erases the whole chip; pass confirm_erase = true after the user confirmed")]
    EraseNotConfirmed,
    #[error("DFU device reports no internal or external flash")]
    NoFlashLayout,
    #[error("cannot parse DFU memory layout {0:?}")]
    LayoutParse(String),
    #[error("firmware does not fit the flash: {len} bytes at {address:#010x}")]
    OutOfRange { address: u32, len: usize },
    #[error("firmware image is empty")]
    EmptyImage,
    #[error("Intel HEX error on line {line}: {reason}")]
    Hex { line: usize, reason: String },
    #[error("verify failed at {address:#010x}: wrote {expected:#04x}, read {actual:#04x}")]
    VerifyMismatch {
        address: u32,
        expected: u8,
        actual: u8,
    },
    #[error("operation cancelled")]
    Cancelled,
    #[error("another flasher operation is using {0}")]
    Busy(String),
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Build API returned HTTP {status} for {url}")]
    HttpStatus { status: u16, url: String },
    #[error("cloud build {key} ended with status {status:?}")]
    BuildFailed { key: String, status: String },
    #[error("firmware not available: {0}")]
    NotCached(String),
    #[error("firmware file changed on disk: expected sha256 {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    /// The image path given to `flasher_flash` is not a file inside the firmware cache.
    #[error("firmware image must be a file in the firmware cache: {0}")]
    PathOutsideCache(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<FlashError> for AppError {
    fn from(e: FlashError) -> Self {
        let msg = e.to_string();
        match e {
            FlashError::Usb(_) => AppError::Usb(msg),
            FlashError::DriverMissing { vid, pid, detail } => {
                AppError::DriverMissing { vid, pid, detail }
            }
            FlashError::DfuNotFound(m) => AppError::DfuNotFound(m),
            FlashError::ReadProtected => AppError::ReadProtected,
            FlashError::EraseNotConfirmed => AppError::EraseConfirmationRequired,
            FlashError::VerifyMismatch { .. } => AppError::VerifyMismatch(msg),
            FlashError::Busy(port) => AppError::PortBusy {
                port,
                held_by: PortHolder::App,
            },
            FlashError::Dfu { .. }
            | FlashError::DfuStuck(..)
            | FlashError::NoFlashLayout
            | FlashError::LayoutParse(_)
            | FlashError::OutOfRange { .. }
            | FlashError::Cancelled => AppError::FlashFailed(msg),
            FlashError::EmptyImage
            | FlashError::Hex { .. }
            | FlashError::Timeout(_)
            | FlashError::Http(_)
            | FlashError::HttpStatus { .. }
            | FlashError::BuildFailed { .. }
            | FlashError::NotCached(_)
            | FlashError::ChecksumMismatch { .. }
            | FlashError::Json(_) => AppError::Firmware(msg),
            FlashError::PathOutsideCache(_) => AppError::InvalidArgument(msg),
            FlashError::Io(_) => AppError::Io(msg),
        }
    }
}

impl From<rusb::Error> for FlashError {
    fn from(e: rusb::Error) -> Self {
        FlashError::Usb(e.to_string())
    }
}

/// Map an error from opening/claiming the DFU device at `vid:pid`. On Windows, "not supported",
/// "access denied" and "not found" from libusb on a device we can see mean no WinUSB driver.
pub(crate) fn open_error(e: rusb::Error, vid: u16, pid: u16) -> FlashError {
    if cfg!(windows)
        && matches!(
            e,
            rusb::Error::NotSupported | rusb::Error::Access | rusb::Error::NotFound
        )
    {
        return FlashError::DriverMissing {
            vid,
            pid,
            detail: e.to_string(),
        };
    }
    if matches!(e, rusb::Error::Access) {
        return FlashError::Usb(format!(
            "{e} (on Linux install the udev rules for {vid:04x}:{pid:04x})"
        ));
    }
    FlashError::Usb(e.to_string())
}

pub type Result<T, E = FlashError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn code(e: FlashError) -> serde_json::Value {
        serde_json::to_value(AppError::from(e)).unwrap()
    }

    #[test]
    fn maps_to_shared_codes() {
        let v = code(FlashError::VerifyMismatch {
            address: 0x0800_0010,
            expected: 1,
            actual: 2,
        });
        assert_eq!(v["code"], "VERIFY_MISMATCH");
        assert!(v["message"].as_str().unwrap().contains("0x08000010"));
        assert_eq!(code(FlashError::ReadProtected)["code"], "READ_PROTECTED");
        assert_eq!(
            code(FlashError::EraseNotConfirmed)["code"],
            "ERASE_CONFIRMATION_REQUIRED"
        );
        assert_eq!(
            code(FlashError::DfuNotFound("none".into()))["code"],
            "DFU_NOT_FOUND"
        );
        assert_eq!(code(FlashError::DfuStuck(100, 4))["code"], "FLASH_FAILED");
        assert_eq!(code(FlashError::Http("dns".into()))["code"], "FIRMWARE");
        assert_eq!(code(FlashError::Usb("pipe".into()))["code"], "USB");
        let busy = code(FlashError::Busy("DFU 0483:df11".into()));
        assert_eq!(busy["code"], "PORT_BUSY");
        assert_eq!(busy["details"]["heldBy"], "APP");
    }

    #[test]
    fn driver_missing_carries_numeric_vid_pid() {
        let v = code(FlashError::DriverMissing {
            vid: 0x2E3C,
            pid: 0xDF11,
            detail: "Operation not supported".into(),
        });
        assert_eq!(v["code"], "DRIVER_MISSING");
        assert_eq!(v["details"]["vid"], 0x2E3C);
        assert_eq!(v["details"]["pid"], 0xDF11);
        assert!(v["message"].as_str().unwrap().contains("2e3c:df11"));
    }
}
