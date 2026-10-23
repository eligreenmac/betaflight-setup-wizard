//! Workstream `device`: host port discovery (USB VID/PID), serial transport,
//! MSP v1/v2 client, CLI mode. Owns the single active FC connection.
//!
//! Public Rust API used by other modules (flasher, config):
//!   - `ConnectionManager` (Tauri managed state, device PR) lends `&mut dyn FcLink`
//!     (`Connection` + `Reconnect`, below) to config/flasher; see ARCHITECTURE.md §4.1.

pub mod commands;
pub mod usb_ids;

use crate::error::AppResult;
use std::time::Duration;

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

/// Re-establish the link after the FC rebooted (`save`, `exit` on 4.4, MSP reboot).
/// Implemented by device's connection; used by config (apply/rollback) and flasher.
pub trait Reconnect {
    /// Close the port, wait (up to `timeout`) for it to disappear and reappear with the
    /// same USB VID/PID/serial number, reopen it and re-verify with MSP_API_VERSION and
    /// MSP_UID (same board). The FC must already be rebooting.
    fn reconnect(&mut self, timeout: Duration) -> AppResult<()>;
}

/// What the connection manager lends to other modules: one link with both capabilities.
pub trait FcLink: Connection + Reconnect {}
impl<T: Connection + Reconnect + ?Sized> FcLink for T {}
