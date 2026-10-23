//! Shared error type. Serialized to the UI as
//! `{ "code": "...", "message": "...", "details"?: {...} }`.
//! `details` is present only for codes that carry structured data (see
//! docs/ARCHITECTURE.md §2.3): `PORT_BUSY` and `DRIVER_MISSING`.

use serde::Serialize;

/// Who holds a serial port when `PORT_BUSY` is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortHolder {
    /// Another operation in this app (the device connection manager allows one owner).
    App,
    /// Another program (Betaflight App, a terminal, ...) or the OS denied access.
    External,
}

impl std::fmt::Display for PortHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PortHolder::App => "in use by another operation in this app",
            PortHolder::External => "in use by another program",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
    #[error("not connected to a flight controller")]
    NotConnected,
    #[error("serial error: {0}")]
    Serial(String),
    #[error("MSP error: {0}")]
    Msp(String),
    #[error("CLI error: {0}")]
    Cli(String),
    #[error("USB/DFU error: {0}")]
    Usb(String),
    #[error("firmware service error: {0}")]
    Firmware(String),
    #[error("configuration error: {0}")]
    Config(String),
    /// Firmware outside the supported lines (4.4.x, 4.5.x, 2025.12.x).
    #[error("unsupported firmware: {0}")]
    UnsupportedFirmware(String),
    #[error("driver error: {0}")]
    Driver(String),
    /// No flight controller / port at the given path (unplugged, wrong COM port).
    #[error("flight controller not found: {0}")]
    BoardNotFound(String),
    /// The serial port is owned by someone else. A second caller inside the app gets
    /// this too (`held_by = App`); there is no separate BUSY code.
    #[error("port {port} is busy ({held_by})")]
    PortBusy { port: String, held_by: PortHolder },
    /// A DFU device is present but cannot be opened (Windows: no WinUSB driver bound).
    /// The UI offers `system_install_dfu_driver` (owned by packaging).
    #[error("USB driver missing for {vid:04x}:{pid:04x}: {detail}")]
    DriverMissing { vid: u16, pid: u16, detail: String },
    /// No device in DFU mode appeared (or the requested serial is not present).
    #[error("no DFU device found: {0}")]
    DfuNotFound(String),
    /// The MCU flash is read-protected. Never removed automatically; the UI must ask
    /// and call `flasher_remove_read_protection` with `confirmErase: true`.
    #[error("the chip is read-protected; removing protection erases the whole chip")]
    ReadProtected,
    /// `flasher_remove_read_protection` was called without `confirmErase: true`.
    #[error("removing read protection erases the whole chip; explicit confirmation is required")]
    EraseConfirmationRequired,
    #[error("flashing failed: {0}")]
    FlashFailed(String),
    #[error("verify failed: {0}")]
    VerifyMismatch(String),
    /// The motor-test safety gate refused (props not confirmed, throttle above the cap,
    /// 3D mode, invalid motor, motors disabled). Motors are at stop.
    #[error("motor test refused: {0}")]
    MotorTestRefused(String),
    /// The FC is armed; motor test refused (or stopped).
    #[error("the flight controller is armed; disarm it first")]
    BoardArmed,
    /// Keepalive/stop without a running motor test (e.g. the watchdog already fired).
    #[error("motor test is not running")]
    MotorTestNotActive,
    /// A command argument is invalid (e.g. a firmware path outside the firmware cache).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("I/O error: {0}")]
    Io(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::NotImplemented(_) => "NOT_IMPLEMENTED",
            AppError::NotConnected => "NOT_CONNECTED",
            AppError::Serial(_) => "SERIAL",
            AppError::Msp(_) => "MSP",
            AppError::Cli(_) => "CLI",
            AppError::Usb(_) => "USB",
            AppError::Firmware(_) => "FIRMWARE",
            AppError::Config(_) => "CONFIG",
            AppError::UnsupportedFirmware(_) => "UNSUPPORTED_FIRMWARE",
            AppError::Driver(_) => "DRIVER",
            AppError::BoardNotFound(_) => "BOARD_NOT_FOUND",
            AppError::PortBusy { .. } => "PORT_BUSY",
            AppError::DriverMissing { .. } => "DRIVER_MISSING",
            AppError::DfuNotFound(_) => "DFU_NOT_FOUND",
            AppError::ReadProtected => "READ_PROTECTED",
            AppError::EraseConfirmationRequired => "ERASE_CONFIRMATION_REQUIRED",
            AppError::FlashFailed(_) => "FLASH_FAILED",
            AppError::VerifyMismatch(_) => "VERIFY_MISMATCH",
            AppError::MotorTestRefused(_) => "MOTOR_TEST_REFUSED",
            AppError::BoardArmed => "BOARD_ARMED",
            AppError::MotorTestNotActive => "MOTOR_TEST_NOT_ACTIVE",
            AppError::InvalidArgument(_) => "INVALID_ARGUMENT",
            AppError::Io(_) => "IO",
        }
    }

    /// The serialized form as a JSON value (used inside event payloads).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({ "code": self.code() }))
    }

    /// Structured payload for the UI, serialized as `details`.
    pub fn details(&self) -> Option<serde_json::Value> {
        match self {
            AppError::PortBusy { port, held_by } => {
                Some(serde_json::json!({ "port": port, "heldBy": held_by }))
            }
            AppError::DriverMissing { vid, pid, detail } => {
                Some(serde_json::json!({ "vid": vid, "pid": pid, "detail": detail }))
            }
            _ => None,
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let details = self.details();
        let mut st = s.serialize_struct("AppError", 2 + usize::from(details.is_some()))?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        if let Some(d) = details {
            st.serialize_field("details", &d)?;
        }
        st.end()
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_errors_have_no_details() {
        let v = serde_json::to_value(AppError::ReadProtected).unwrap();
        assert_eq!(v["code"], "READ_PROTECTED");
        assert!(v.get("details").is_none());
    }

    #[test]
    fn driver_missing_carries_vid_pid() {
        let e = AppError::DriverMissing {
            vid: 0x2E3C,
            pid: 0xDF11,
            detail: "Operation not supported".into(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "DRIVER_MISSING");
        assert!(v["message"].as_str().unwrap().contains("2e3c:df11"));
        assert_eq!(
            v["details"],
            json!({ "vid": 0x2E3C, "pid": 0xDF11, "detail": "Operation not supported" })
        );
    }

    #[test]
    fn port_busy_says_who_holds_it() {
        let e = AppError::PortBusy {
            port: "COM5".into(),
            held_by: PortHolder::App,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "PORT_BUSY");
        assert_eq!(v["details"], json!({ "port": "COM5", "heldBy": "APP" }));
    }
}
