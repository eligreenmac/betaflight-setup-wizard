//! Serial port function bitmask, from betaflight src/main/io/serial.h
//! (stable across 4.4 .. 2025.12+).

pub const MSP: u32 = 1 << 0;
pub const GPS: u32 = 1 << 1;
pub const TELEMETRY_FRSKY_HUB: u32 = 1 << 2;
pub const TELEMETRY_HOTT: u32 = 1 << 3;
pub const TELEMETRY_LTM: u32 = 1 << 4;
pub const TELEMETRY_SMARTPORT: u32 = 1 << 5;
pub const RX_SERIAL: u32 = 1 << 6;
pub const BLACKBOX: u32 = 1 << 7;
pub const TELEMETRY_MAVLINK: u32 = 1 << 9;
pub const ESC_SENSOR: u32 = 1 << 10;
pub const VTX_SMARTAUDIO: u32 = 1 << 11;
pub const TELEMETRY_IBUS: u32 = 1 << 12;
pub const VTX_TRAMP: u32 = 1 << 13;
pub const RCDEVICE: u32 = 1 << 14;
pub const LIDAR: u32 = 1 << 15;
pub const FRSKY_OSD: u32 = 1 << 16;
pub const VTX_MSP: u32 = 1 << 17;
