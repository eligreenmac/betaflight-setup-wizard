// SPDX-License-Identifier: GPL-3.0-or-later
//! Internal error type of the device module. Mapped to the shared `AppError` at the
//! command boundary (`From<DeviceError> for AppError`).

use crate::error::{AppError, PortHolder};
use std::time::Duration;

/// Why the motor-test safety gate said no.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MotorTestRefusal {
    #[error("confirm that all propellers are removed first")]
    PropsNotConfirmed,
    #[error("throttle {requested} is above the motor-test limit {max}")]
    ThrottleAboveLimit { requested: u16, max: u16 },
    #[error("throttle {requested} is below the motor stop value {stop}")]
    ThrottleBelowStop { requested: u16, stop: u16 },
    #[error("motor index {index} out of range (board has {count} motors)")]
    InvalidMotor { index: u8, count: u8 },
    #[error("3D mode (reversible motors) is enabled; the wizard does not motor-test in 3D mode")]
    Mode3d,
    #[error("motor output is disabled (no ESC protocol configured)")]
    MotorsDisabled,
}

/// Everything that can go wrong talking to a flight controller.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("serial port error: {0}")]
    Serial(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("timed out after {after:?} waiting for {what}")]
    Timeout { what: String, after: Duration },
    #[error("port {0} not found")]
    NotFound(String),
    #[error("access denied to {0} (port in use by another app, or missing permission)")]
    AccessDenied(String),
    #[error("port disconnected")]
    Disconnected,
    #[error("MSP v1 cannot carry command code {0} (>255); use MSP v2")]
    MspCodeTooLarge(u16),
    #[error("MSP command {0} is not supported by the firmware")]
    MspUnsupported(u16),
    #[error("MSP response for command {code} too short: needed {needed} bytes, got {got}")]
    MspPayloadTooShort {
        code: u16,
        needed: usize,
        got: usize,
    },
    #[error("malformed MSP response: {0}")]
    MspMalformed(String),
    #[error("unexpected CLI response: {0}")]
    Cli(String),
    #[error("motor test refused: {0}")]
    MotorTestRefused(MotorTestRefusal),
    #[error("the flight controller is armed; disarm it first")]
    Armed,
    #[error("motor test is not running")]
    MotorTestNotActive,
    #[error("no usable accelerometer (present: {present}, enabled: {enabled})")]
    AccNotAvailable { present: bool, enabled: bool },
    #[error("the board is moving ({max_rate_dps} deg/s > {limit_dps} deg/s)")]
    BoardMoving {
        max_rate_dps: u16,
        limit_dps: u16,
        during: bool,
    },
    #[error("accelerometer calibration failed: {0}")]
    AccCalibrationFailed(String),
}

impl DeviceError {
    pub(crate) fn timeout(what: impl Into<String>, after: Duration) -> Self {
        DeviceError::Timeout {
            what: what.into(),
            after,
        }
    }
}

impl From<serialport::Error> for DeviceError {
    fn from(e: serialport::Error) -> Self {
        match e.kind {
            serialport::ErrorKind::NoDevice => DeviceError::NotFound(e.description),
            serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied) => {
                DeviceError::AccessDenied(e.description)
            }
            serialport::ErrorKind::Io(std::io::ErrorKind::NotFound) => {
                DeviceError::NotFound(e.description)
            }
            _ => DeviceError::Serial(e.description),
        }
    }
}

impl From<DeviceError> for AppError {
    fn from(e: DeviceError) -> Self {
        let msg = e.to_string();
        match e {
            DeviceError::NotFound(p) => AppError::BoardNotFound(p),
            DeviceError::AccessDenied(port) => AppError::PortBusy {
                port,
                held_by: PortHolder::External,
            },
            DeviceError::Serial(_) | DeviceError::Timeout { .. } | DeviceError::Disconnected => {
                AppError::Serial(msg)
            }
            DeviceError::Io(io) => AppError::Io(io.to_string()),
            DeviceError::MspCodeTooLarge(_)
            | DeviceError::MspUnsupported(_)
            | DeviceError::MspPayloadTooShort { .. }
            | DeviceError::MspMalformed(_) => AppError::Msp(msg),
            DeviceError::Cli(m) => AppError::Cli(m),
            DeviceError::MotorTestRefused(r) => AppError::MotorTestRefused(r.to_string()),
            DeviceError::Armed => AppError::BoardArmed,
            DeviceError::MotorTestNotActive => AppError::MotorTestNotActive,
            DeviceError::AccNotAvailable { present, enabled } => {
                AppError::AccNotAvailable { present, enabled }
            }
            DeviceError::BoardMoving {
                max_rate_dps,
                limit_dps,
                during,
            } => AppError::BoardMoving {
                max_rate_dps,
                limit_dps,
                during,
            },
            DeviceError::AccCalibrationFailed(m) => AppError::AccCalibrationFailed(m),
        }
    }
}

pub type Result<T, E = DeviceError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_to_shared_codes() {
        let c = |e: DeviceError| AppError::from(e).code();
        assert_eq!(c(DeviceError::NotFound("COM9".into())), "BOARD_NOT_FOUND");
        assert_eq!(c(DeviceError::AccessDenied("COM9".into())), "PORT_BUSY");
        assert_eq!(c(DeviceError::Disconnected), "SERIAL");
        assert_eq!(c(DeviceError::MspUnsupported(1)), "MSP");
        assert_eq!(c(DeviceError::Cli("x".into())), "CLI");
        assert_eq!(
            c(DeviceError::MotorTestRefused(
                MotorTestRefusal::PropsNotConfirmed
            )),
            "MOTOR_TEST_REFUSED"
        );
        assert_eq!(c(DeviceError::Armed), "BOARD_ARMED");
        assert_eq!(c(DeviceError::MotorTestNotActive), "MOTOR_TEST_NOT_ACTIVE");
        assert_eq!(
            c(DeviceError::AccNotAvailable {
                present: false,
                enabled: true
            }),
            "ACC_NOT_AVAILABLE"
        );
        assert_eq!(
            c(DeviceError::BoardMoving {
                max_rate_dps: 30,
                limit_dps: 8,
                during: false
            }),
            "BOARD_MOVING"
        );
        assert_eq!(
            c(DeviceError::AccCalibrationFailed("x".into())),
            "ACC_CALIBRATION_FAILED"
        );
        let busy = AppError::from(DeviceError::AccessDenied("COM9".into()));
        assert_eq!(
            busy.details().unwrap(),
            serde_json::json!({"port": "COM9", "heldBy": "EXTERNAL"})
        );
    }
}
