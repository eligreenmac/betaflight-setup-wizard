// SPDX-License-Identifier: GPL-3.0-or-later
//! Workstream `flasher`: Build API client + firmware cache, STM32/AT32 USB DFU
//! flashing, progress events.
//!
//! Progress is emitted on the Tauri event `flasher://progress` with payload
//! `model::FlashProgress`.
//!
//! Parts of this module are ported from Betaflight Configurator
//! (https://github.com/betaflight/betaflight-configurator, GPL-3.0-or-later):
//! `src/js/protocols/usbdfu.ts` (DfuSe), `src/js/workers/hex_parser.ts` (Intel HEX),
//! `src/js/BuildApi.ts` and `src/composables/useCloudBuild.ts` (Build API + polling).

pub mod build_api;
pub mod build_types;
pub mod cache;
pub mod commands;
pub mod dfu;
pub mod error;
pub mod hex;
pub mod layout;
pub mod service;
pub mod usb;

/// Base URL of the Betaflight cloud build service (firmware 4.4+).
pub const BUILD_API_URL: &str = build_api::DEFAULT_BASE_URL;
/// Event name for flashing progress.
pub const PROGRESS_EVENT: &str = "flasher://progress";
