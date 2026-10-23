//! Workstream `config`: DroneModel -> firmware-specific CLI script (pure),
//! backup (`diff all`), apply (batch + save), verify (read-back), rollback.
//!
//! `generate` must be a pure function so it can be golden-tested without hardware.

pub mod commands;
pub mod serial_functions;

use crate::error::{AppError, AppResult};
use crate::model::{CliScript, DroneModel, FcSerialPort};

/// Pure: build the CLI script for `model` given the ports read from the FC.
pub fn generate(model: &DroneModel, ports: &[FcSerialPort]) -> AppResult<CliScript> {
    let _ = (model, ports);
    Err(AppError::NotImplemented("config::generate"))
}
