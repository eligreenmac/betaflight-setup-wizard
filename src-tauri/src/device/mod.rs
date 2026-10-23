//! Workstream `device`: host port discovery (USB VID/PID), serial transport,
//! MSP v1/v2 client, CLI mode. Owns the single active FC connection.
//!
//! Public Rust API used by other modules (flasher, config):
//!   - `Connection` trait (below) - config/flasher get `&mut dyn Connection`.

pub mod commands;
pub mod usb_ids;

use crate::error::AppResult;

/// Payload `MotorTestStopped`.
pub const MOTOR_TEST_STOPPED_EVENT: &str = "device://motor-test-stopped";
/// Payload `RcChannels`.
pub const RC_CHANNELS_EVENT: &str = "device://rc-channels";
/// Payload `RcStopped`.
pub const RC_STOPPED_EVENT: &str = "device://rc-stopped";

/// Abstraction over an open FC link so config/flasher can be unit-tested
/// against a mock (see config tests).
pub trait Connection: Send {
    /// Send one MSP request and wait for its reply payload.
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>>;
    /// Enter CLI mode (send `#`) if not already in it.
    fn enter_cli(&mut self) -> AppResult<()>;
    /// Send one CLI line and return the raw response up to the next prompt.
    fn cli_exec(&mut self, line: &str) -> AppResult<String>;
    /// Leave CLI; `save` = true sends `save` (FC reboots), else `exit`.
    fn exit_cli(&mut self, save: bool) -> AppResult<()>;
}
