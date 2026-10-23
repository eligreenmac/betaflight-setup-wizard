//! Shared error type. Serialized to the UI as
//! `{ "code": "...", "message": "...", "details"?: {...} }`.
//! `details` is present only for codes that carry structured data (see
//! docs/ARCHITECTURE.md §2.3): `PORT_BUSY`, `DRIVER_MISSING`, `ACC_NOT_AVAILABLE`,
//! `BOARD_MOVING` and `PENDING_UNSAVED_CHANGES`.

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

/// Why the flight controller's RAM may hold changes that are not in its EEPROM
/// (`PENDING_UNSAVED_CHANGES`, ARCHITECTURE.md §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnsavedChangesReason {
    /// A CLI command other than a read-only one (`version`, `dump`, `diff`, `get`, ...) was sent.
    CliCommand,
    /// The CLI was left with `exit noreboot` (4.5+), which keeps CLI changes live in RAM.
    #[serde(rename = "EXIT_NOREBOOT")]
    ExitNoReboot,
    /// An MSP command that may change settings (`MSP_SET_*`, ...) was sent.
    MspWrite,
}

impl std::fmt::Display for UnsavedChangesReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            UnsavedChangesReason::CliCommand => "a CLI command that can change settings was sent",
            UnsavedChangesReason::ExitNoReboot => "the CLI was left without a reboot",
            UnsavedChangesReason::MspWrite => "a setting may have been written over MSP",
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
    /// Accelerometer calibration impossible: no accelerometer detected, or it is disabled
    /// (`acc_hardware = NONE`), or the firmware was built without one.
    #[error("no usable accelerometer (present: {present}, enabled: {enabled})")]
    AccNotAvailable { present: bool, enabled: bool },
    /// The gyro shows rotation above `limit_dps` (deg/s): the board is being handled.
    /// `during = false`: checked before calibrating, nothing was sent. `during = true`: the
    /// board moved while the FC calibrated; the (bad) result is already stored, calibrate
    /// again with the board still.
    #[error("the board is moving ({max_rate_dps} deg/s > {limit_dps} deg/s); keep it level and still{}", if *during { " and calibrate again" } else { "" })]
    BoardMoving {
        max_rate_dps: u16,
        limit_dps: u16,
        during: bool,
    },
    /// The FC did not report a finished accelerometer calibration in time.
    #[error("accelerometer calibration failed: {0}")]
    AccCalibrationFailed(String),
    /// The FC may hold unsaved changes in RAM (e.g. a config apply failed and its cleanup
    /// reboot was not confirmed). Refused because the operation (accelerometer calibration)
    /// saves the whole RAM configuration to EEPROM. Nothing was sent; a confirmed reboot or a
    /// reconnect clears it.
    #[error("the flight controller may have unsaved changes ({reason}); reconnect or reboot the board first")]
    PendingUnsavedChanges { reason: UnsavedChangesReason },
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
            AppError::AccNotAvailable { .. } => "ACC_NOT_AVAILABLE",
            AppError::BoardMoving { .. } => "BOARD_MOVING",
            AppError::AccCalibrationFailed(_) => "ACC_CALIBRATION_FAILED",
            AppError::PendingUnsavedChanges { .. } => "PENDING_UNSAVED_CHANGES",
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
            AppError::AccNotAvailable { present, enabled } => {
                Some(serde_json::json!({ "present": present, "enabled": enabled }))
            }
            AppError::BoardMoving {
                max_rate_dps,
                limit_dps,
                during,
            } => Some(serde_json::json!({
                "maxRateDps": max_rate_dps,
                "limitDps": limit_dps,
                "during": during,
            })),
            AppError::PendingUnsavedChanges { reason } => {
                Some(serde_json::json!({ "reason": reason }))
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

    #[test]
    fn sensor_errors_carry_details() {
        let v = serde_json::to_value(AppError::AccNotAvailable {
            present: true,
            enabled: false,
        })
        .unwrap();
        assert_eq!(v["code"], "ACC_NOT_AVAILABLE");
        assert_eq!(v["details"], json!({ "present": true, "enabled": false }));
        let v = serde_json::to_value(AppError::BoardMoving {
            max_rate_dps: 42,
            limit_dps: 8,
            during: true,
        })
        .unwrap();
        assert_eq!(v["code"], "BOARD_MOVING");
        assert_eq!(
            v["details"],
            json!({ "maxRateDps": 42, "limitDps": 8, "during": true })
        );
        assert!(v["message"].as_str().unwrap().ends_with("calibrate again"));
        let v = serde_json::to_value(AppError::PendingUnsavedChanges {
            reason: UnsavedChangesReason::ExitNoReboot,
        })
        .unwrap();
        assert_eq!(v["code"], "PENDING_UNSAVED_CHANGES");
        assert_eq!(v["details"], json!({ "reason": "EXIT_NOREBOOT" }));
        assert!(v["message"]
            .as_str()
            .unwrap()
            .ends_with("reconnect or reboot the board first"));
        for (r, s) in [
            (UnsavedChangesReason::CliCommand, "CLI_COMMAND"),
            (UnsavedChangesReason::MspWrite, "MSP_WRITE"),
        ] {
            assert_eq!(serde_json::to_value(r).unwrap(), json!(s));
        }
    }

    /// `src/types/model.ts` (`AppErrorCode`) is the TypeScript source of truth for codes;
    /// every Rust code must appear in it.
    #[test]
    fn every_code_is_in_the_typescript_union() {
        let ts = include_str!("../../src/types/model.ts");
        let start = ts
            .find("export type AppErrorCode =")
            .expect("AppErrorCode union");
        let union = &ts[start..start + ts[start..].find(';').expect("end of union")];
        let all = [
            AppError::NotImplemented("x"),
            AppError::NotConnected,
            AppError::Serial(String::new()),
            AppError::Msp(String::new()),
            AppError::Cli(String::new()),
            AppError::Usb(String::new()),
            AppError::Firmware(String::new()),
            AppError::Config(String::new()),
            AppError::UnsupportedFirmware(String::new()),
            AppError::Driver(String::new()),
            AppError::BoardNotFound(String::new()),
            AppError::PortBusy {
                port: String::new(),
                held_by: PortHolder::App,
            },
            AppError::DriverMissing {
                vid: 0,
                pid: 0,
                detail: String::new(),
            },
            AppError::DfuNotFound(String::new()),
            AppError::ReadProtected,
            AppError::EraseConfirmationRequired,
            AppError::FlashFailed(String::new()),
            AppError::VerifyMismatch(String::new()),
            AppError::MotorTestRefused(String::new()),
            AppError::BoardArmed,
            AppError::MotorTestNotActive,
            AppError::AccNotAvailable {
                present: false,
                enabled: false,
            },
            AppError::BoardMoving {
                max_rate_dps: 0,
                limit_dps: 0,
                during: false,
            },
            AppError::AccCalibrationFailed(String::new()),
            AppError::PendingUnsavedChanges {
                reason: UnsavedChangesReason::CliCommand,
            },
            AppError::InvalidArgument(String::new()),
            AppError::Io(String::new()),
        ];
        for e in all {
            let quoted = format!("\"{}\"", e.code());
            assert!(
                union.contains(&quoted),
                "{} missing from AppErrorCode",
                e.code()
            );
        }
    }
}
