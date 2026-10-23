//! Workstream `config`: DroneModel -> firmware-specific CLI script (pure),
//! backup (`diff all`), apply (batch + save), verify (read-back), rollback.
//!
//! `generate` must be a pure function so it can be golden-tested without hardware.
//!
//! Layout:
//! - [`firmware`]: version parsing and the 4.4 / 4.5 / 2025.12 support gate
//! - [`serial_functions`]: function bits, port ids per dialect, `isSerialConfigValid` mirror
//! - [`snapshot`]: parsers for `serial`, `dump all`, `diff all`
//! - [`modes`]: managed flight modes -> permanent box ids
//! - [`generate`]: the per-dialect generator (also used by plan and verify)
//! - [`session`]: FC operations over `device::Connection` (read ports, plan, backup, apply, verify, rollback)
//! - [`backup`]: backup files on disk
//! - [`guard`]: allowlists for what apply / rollback may send (script and backup replay)
//! - `mock` (tests only): `MockFc` CLI simulator

pub mod backup;
pub mod commands;
pub mod firmware;
pub mod generate;
pub mod guard;
pub mod md5;
pub mod modes;
pub mod serial_functions;
pub mod session;
pub mod snapshot;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

use crate::error::AppResult;
use crate::model::{CliScript, DroneModel, FcSerialPort};
use firmware::Dialect;
use snapshot::PortState;

/// Pure: build the CLI script for `model` given the ports read from the FC.
/// **Preview only** (`config_generate`): the script that gets applied is always the one
/// from `session::plan` (`config_plan`).
///
/// The dialect comes from `model.fc.firmware_version` (UNSUPPORTED_FIRMWARE outside
/// 4.4.x / 4.5.x / 2025.12.x). Without a `dump all` snapshot the generator assumes
/// default baud rates, all aux slots free and does not check SPI receiver hardware;
/// `config_plan` (session::plan) uses the live FC state instead.
pub fn generate(model: &DroneModel, ports: &[FcSerialPort]) -> AppResult<CliScript> {
    let dialect = Dialect::from_version(&model.fc.firmware_version)?;
    let roles = [
        &model.receiver.port,
        &model.vtx.port,
        &model.gps.port,
        &model.esc.telemetry_port,
    ];
    for p in ports.iter().filter(|p| p.is_connection_port) {
        if roles.iter().any(|r| r.as_deref() == Some(p.name.as_str())) {
            return Err(crate::error::AppError::Config(format!(
                "{} is the port the app is connected through; pick another port",
                p.name
            )));
        }
    }
    let ports: Vec<PortState> = ports.iter().map(PortState::from_fc_port).collect();
    generate::generate_for(model, dialect, &ports, None)
}
