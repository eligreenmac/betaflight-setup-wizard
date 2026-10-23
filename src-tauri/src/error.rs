//! Shared error type. Serialized to the UI as `{ "code": "...", "message": "..." }`.

use serde::Serialize;

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
            AppError::Io(_) => "IO",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("AppError", 2)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
