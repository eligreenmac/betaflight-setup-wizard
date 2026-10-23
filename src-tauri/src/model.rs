//! Shared data types crossing module and UI boundaries.
//!
//! These types are the Rust side of `schemas/drone-model.schema.json` and
//! `src/types/model.ts`. All three MUST stay in sync; change them only in a
//! PR labelled `shared` that updates all three files together.
//!
//! Serialization: camelCase field names, enums as SCREAMING_SNAKE_CASE strings
//! unless noted.

use serde::{Deserialize, Serialize};

pub const DRONE_MODEL_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------- device

/// A serial/USB port seen on the host (Windows COM port).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPort {
    /// e.g. "COM5"
    pub path: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    /// Classification from the VID/PID table in device::usb_ids.
    pub kind: HostPortKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HostPortKind {
    /// Native MCU USB virtual COM port (STM32/AT32/APM32/GD32/RP2040 VCP).
    FcVcp,
    /// USB-UART bridge (CP210x, CH340, FTDI) - FC or external adapter.
    UsbUartBridge,
    Unknown,
}

/// Identity of a flight controller running Betaflight, read via MSP.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FcInfo {
    /// MSP_FC_VARIANT, e.g. "BTFL"
    pub fc_variant: String,
    /// MSP_FC_VERSION, e.g. "4.5.2" or "2025.12.1"
    pub firmware_version: String,
    /// MSP_API_VERSION, e.g. "1.46"
    pub api_version: String,
    /// MSP_BOARD_INFO board name / Build API target, e.g. "SPEEDYBEEF405V4"
    pub board_name: String,
    /// Manufacturer id from MSP_BOARD_INFO, e.g. "SPBE"
    pub manufacturer_id: Option<String>,
    /// MCU type, e.g. "STM32F405"
    pub mcu: Option<String>,
    /// Hex-encoded unique id (MSP_UID)
    pub uid: Option<String>,
    /// Build options/defines compiled in (from Build API key or MSP_BUILD_INFO)
    pub build_options: Vec<String>,
}

/// A flight-controller-side serial port (UART pad), read from the FC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FcSerialPort {
    /// Stable name used by the wizard and drone model: "UART1".."UART15",
    /// "LPUART1", "SOFTSERIAL1", "SOFTSERIAL2", "VCP".
    pub name: String,
    /// Firmware-specific numeric identifier used in the `serial` CLI command.
    /// NOTE: differs by firmware version (4.4/4.5: UART1=0; 2025.12+: UART1=51).
    /// Never hard-code - always take it from config_read_ports.
    pub identifier: i32,
    /// Current function bitmask (FUNCTION_* in betaflight src/main/io/serial.h).
    pub function_mask: u32,
    /// True for the port the app is connected through (USB VCP) - not offered to the user.
    pub is_connection_port: bool,
}

// ---------------------------------------------------------------- flasher

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardTarget {
    pub target: String,
    pub manufacturer: Option<String>,
    pub mcu: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareRelease {
    /// e.g. "4.5.2", "2025.12.1"
    pub release: String,
    pub prerelease: bool,
}

/// What to build/fetch. Options are Build API option keys (e.g. "USE_GPS").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareRequest {
    pub target: String,
    pub release: String,
    pub options: Vec<String>,
    /// If true, never touch the network; fail if not cached.
    pub offline_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareImage {
    pub request: FirmwareRequest,
    /// Absolute path of the cached .hex/.bin/.uf2 file.
    pub path: String,
    pub format: FirmwareFormat,
    pub sha256: String,
    /// Build API build key, when built in the cloud.
    pub build_key: Option<String>,
    pub from_cache: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FirmwareFormat {
    Hex,
    Bin,
    Uf2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DfuDevice {
    pub vid: u16,
    pub pid: u16,
    pub serial: Option<String>,
    /// Parsed from the DFU interface string, e.g. "STM32F405"/"AT32F435" family hint.
    pub mcu_family: Option<String>,
    /// Raw alt-setting descriptor, e.g. "@Internal Flash  /0x08000000/04*016Kg,..."
    pub flash_descriptor: Option<String>,
    /// False when Windows has no WinUSB driver bound (needs system_install_dfu_driver).
    pub driver_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashOptions {
    /// Full chip erase (required for most target changes). Default true.
    pub full_erase: bool,
    /// Reboot into firmware when done. Default true.
    pub manifest: bool,
}

/// Emitted on event `flasher://progress`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlashProgress {
    pub stage: FlashStage,
    /// 0.0..=1.0 within the stage
    pub fraction: f32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FlashStage {
    Downloading,
    EnteringBootloader,
    WaitingForDevice,
    Erasing,
    Writing,
    Verifying,
    Rebooting,
    Done,
    Failed,
}

// ---------------------------------------------------------------- drone model

/// The user's answers, independent of firmware version. Produced by the
/// wizard UI, consumed by config::generate. See schemas/drone-model.schema.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DroneModel {
    pub schema_version: u32,
    /// Target FC as detected (board name + firmware version drive CLI dialect).
    pub fc: FcRef,
    pub receiver: Receiver,
    pub vtx: Vtx,
    pub gps: Gps,
    pub esc: Esc,
    #[serde(default)]
    pub switches: Vec<SwitchAssignment>,
    #[serde(default)]
    pub extras: Extras,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FcRef {
    pub board_name: String,
    pub firmware_version: String,
    pub api_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receiver {
    pub kind: ReceiverKind,
    /// Serial RX provider (when kind = SERIAL) or SPI protocol (when kind = SPI).
    pub protocol: Option<RxProtocol>,
    /// FC port name, e.g. "UART2". Required when kind = SERIAL.
    pub port: Option<String>,
    #[serde(default)]
    pub telemetry: bool,
    #[serde(default)]
    pub inverted: bool,
    #[serde(default)]
    pub half_duplex: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReceiverKind {
    None,
    Serial,
    Spi,
    Ppm,
    Msp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RxProtocol {
    /// ExpressLRS / TBS Crossfire / Tracer over UART
    Crsf,
    Ghst,
    Sbus,
    Fport,
    Ibus,
    Srxl2,
    Spek1024,
    Spek2048,
    Sumd,
    /// Built-in SPI ExpressLRS receiver
    Expresslrs,
    /// Built-in SPI FrSky
    FrskyD,
    FrskyX,
    FrskyXLbt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vtx {
    pub kind: VtxKind,
    pub protocol: Option<VtxProtocol>,
    /// FC port name, e.g. "UART1"
    pub port: Option<String>,
    /// Digital system hint (affects OSD/canvas settings).
    pub digital_system: Option<DigitalSystem>,
    /// ISO-3166 region code for analog VTX tables / power legality, e.g. "IL".
    pub region: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VtxKind {
    None,
    Analog,
    Digital,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VtxProtocol {
    Smartaudio,
    Tramp,
    /// MSP / MSP DisplayPort (digital systems, some analog VTX)
    Msp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DigitalSystem {
    Dji,
    Walksnail,
    Hdzero,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gps {
    pub enabled: bool,
    pub port: Option<String>,
    pub provider: Option<GpsProvider>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GpsProvider {
    Ublox,
    Nmea,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Esc {
    pub protocol: EscProtocol,
    #[serde(default)]
    pub bidirectional_dshot: bool,
    /// Magnet poles, typically 14 (5" motors) or 12.
    pub motor_poles: Option<u8>,
    /// Port for analog ESC telemetry wire, if used.
    pub telemetry_port: Option<String>,
    /// Props-out ("reversed") motor direction.
    #[serde(default)]
    pub props_out: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EscProtocol {
    Dshot150,
    Dshot300,
    Dshot600,
    Oneshot125,
    Multishot,
    Pwm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchAssignment {
    /// Betaflight mode name, e.g. "ARM", "ANGLE", "BEEPER", "FLIP OVER AFTER CRASH"
    pub mode: String,
    /// AUX channel index, 0 = AUX1 (channel 5)
    pub aux_channel: u8,
    /// PWM range, 900..2100, step 25
    pub range_min: u16,
    pub range_max: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extras {
    pub beeper: Option<bool>,
    pub led_strip: Option<bool>,
    /// Name shown in OSD / Configurator
    pub craft_name: Option<String>,
}

// ---------------------------------------------------------------- config engine

/// Firmware-version-specific CLI script produced by config::generate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliScript {
    /// Commands WITHOUT the surrounding `batch start`/`batch end`/`save`
    /// (config_apply adds those).
    pub commands: Vec<String>,
    /// Non-fatal issues to show the user (e.g. "UART3 also used by GPS").
    pub warnings: Vec<String>,
    /// Firmware dialect used, e.g. "4.5" or "2025.12".
    pub dialect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    /// Absolute path of the saved `diff all` text file.
    pub path: String,
    pub board_name: String,
    pub firmware_version: String,
    /// RFC 3339 timestamp.
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub applied: usize,
    pub errors: Vec<CliError>,
    pub saved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliError {
    pub command: String,
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub ok: bool,
    pub mismatches: Vec<Mismatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mismatch {
    pub setting: String,
    pub expected: String,
    pub actual: String,
}

// ---------------------------------------------------------------- system

/// Result of `system_driver_status`. On non-Windows hosts everything is `true`
/// and `devices` is empty (no driver installation is ever needed there).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverStatus {
    /// False only when a DFU device (STM32 0483:DF11, AT32 2E3C:DF11, ...) is
    /// plugged in WITHOUT WinUSB bound -> UI offers `system_install_dfu_driver`.
    /// True when every connected DFU device is OK or none is connected.
    pub dfu_winusb: bool,
    /// False only when a FC VCP or USB-UART bridge (CP210x, CH340, ...) is
    /// plugged in without a working driver -> UI shows `guidance` for it.
    pub vcp: bool,
    /// Human-readable diagnostics (English, for logs/support; not for i18n UI).
    pub notes: Vec<String>,
    /// Every connected USB device we care about, with its driver binding.
    #[serde(default)]
    pub devices: Vec<UsbDriverInfo>,
    /// Official vendor driver downloads for USB-UART bridges, always filled
    /// (for the "nothing detected -> driver help" screen). Never bundled.
    #[serde(default)]
    pub guidance: Vec<DriverGuidance>,
}

/// A connected USB device relevant to the wizard and its Windows driver state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDriverInfo {
    pub vid: u16,
    pub pid: u16,
    /// Friendly name from our VID/PID table, e.g. "STM32 DFU", "CP210x".
    pub name: String,
    pub kind: UsbDeviceKind,
    /// Windows device instance id, e.g. `USB\VID_0483&PID_DF11\205F32...`.
    pub instance_id: Option<String>,
    /// Bound driver service, e.g. "WinUSB", "silabser", "CH341SER_A64",
    /// "usbser"; `None` when Windows has no driver bound.
    pub service: Option<String>,
    /// True when the bound driver is the one the wizard needs.
    pub driver_ok: bool,
    /// What the user should do when `driver_ok` is false.
    pub action: DriverAction,
    /// For `action = VENDOR_DOWNLOAD`: index into `DriverStatus.guidance`.
    pub guidance_index: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UsbDeviceKind {
    /// MCU ROM USB DFU bootloader (needs WinUSB).
    Dfu,
    /// Native MCU USB virtual COM port (inbox usbser on Windows 10+).
    FcVcp,
    /// USB-UART bridge chip (CP210x, CH340, ...) - needs the vendor driver.
    UsbUartBridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DriverAction {
    /// Driver is fine.
    None,
    /// Call `system_install_dfu_driver` (one UAC prompt).
    InstallDfuDriver,
    /// Send the user to the official vendor download (`guidance_index`).
    VendorDownload,
    /// Unplug/replug or check Device Manager (e.g. device in an error state).
    Replug,
}

/// Official driver download for a USB-UART bridge family.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverGuidance {
    /// Stable id usable as an i18n key suffix: "CP210X", "CH34X", "CH343".
    pub id: String,
    /// e.g. "Silicon Labs"
    pub vendor: String,
    /// e.g. "CP210x (CP2102/CP2104/CP2102N...)"
    pub chips: String,
    /// "VID:PID" strings (upper-case hex) this guidance applies to.
    pub usb_ids: Vec<String>,
    /// Official vendor download page (https).
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub os: String,
    pub cache_dir: String,
}
