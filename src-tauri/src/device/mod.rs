// SPDX-License-Identifier: GPL-3.0-or-later
//! Workstream `device`: host port discovery (USB VID/PID), serial transport,
//! MSP v1/v2 client, CLI mode. Owns the single active FC connection.
//!
//! Public Rust API used by other modules (flasher, config):
//!   - `ConnectionManager` (Tauri managed state, device PR) lends `&mut dyn FcLink`
//!     (`Connection` + `Reconnect`, below) to config/flasher; see ARCHITECTURE.md §4.1.

pub mod arming;
pub mod attitude;
pub mod build_info;
pub mod cli;
#[cfg(test)]
mod cli_state_tests;
pub mod commands;
#[cfg(test)]
mod dirty_ram_tests;
pub mod error;
pub mod manager;
#[cfg(test)]
pub(crate) mod mock;
pub mod motor;
pub mod msp;
pub mod ram_state;
pub mod rc;
pub mod rx_spi;
pub mod sensors;
pub mod serial;
pub mod serial_ports;
#[cfg(test)]
mod sitl_smoke_tests;
pub mod usb_ids;

pub use manager::ConnectionManager;

use crate::error::AppResult;
use std::time::Duration;

/// Payload `MotorTestStopped`.
pub const MOTOR_TEST_STOPPED_EVENT: &str = "device://motor-test-stopped";
/// Payload `RcChannels`.
pub const RC_CHANNELS_EVENT: &str = "device://rc-channels";
/// Payload `RcStopped`.
pub const RC_STOPPED_EVENT: &str = "device://rc-stopped";
/// Payload `Attitude`.
pub const ATTITUDE_EVENT: &str = "device://attitude";
/// Payload `AttitudeStopped`.
pub const ATTITUDE_STOPPED_EVENT: &str = "device://attitude-stopped";

/// Abstraction over an open FC link so config/flasher can be unit-tested
/// against a mock (see config tests).
pub trait Connection: Send {
    /// Send one MSP request and wait for its reply payload.
    fn msp_request(&mut self, code: u16, payload: &[u8]) -> AppResult<Vec<u8>>;
    /// Enter CLI mode (send `#`) if not already in it. The device connection remembers that
    /// the CLI is open: the next MSP call (here or from identify / motor test / RC stream)
    /// first leaves it with plain `exit` (the FC reboots, unsaved changes are dropped) and
    /// reconnects, verifying MSP_UID. `save` is only ever sent by `exit_cli(true)`.
    fn enter_cli(&mut self) -> AppResult<()>;
    /// Send one CLI line and return the raw response up to the next prompt.
    fn cli_exec(&mut self, line: &str) -> AppResult<String>;
    /// Leave CLI; `save` = true sends `save` (FC reboots), else `exit`.
    fn exit_cli(&mut self, save: bool) -> AppResult<()>;
    /// The board's MSP_UID as the CLI `mcu_id` string (`FcInfo.uid`), cached by the device
    /// connection (read on connect, re-verified on every reconnect): no round-trip, works
    /// in CLI mode. `None` if unknown (MSP_UID unsupported) or for test doubles.
    fn uid(&self) -> Option<String> {
        None
    }
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
