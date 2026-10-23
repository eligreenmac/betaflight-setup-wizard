//! Workstream `flasher`: Build API client + firmware cache, STM32/AT32 USB DFU
//! flashing, bootloader entry orchestration, progress events.
//!
//! Progress is emitted on the Tauri event `flasher://progress` with payload
//! `model::FlashProgress`.

pub mod commands;

/// Base URL of the Betaflight cloud build service (firmware 4.4+).
pub const BUILD_API_URL: &str = "https://build.betaflight.com";
/// Event name for flashing progress.
pub const PROGRESS_EVENT: &str = "flasher://progress";
