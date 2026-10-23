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
    /// FC port name, e.g. "UART2". Required when kind = SERIAL, must be null for SPI.
    pub port: Option<String>,
    /// Enable `feature TELEMETRY` for links that carry it (CRSF, FPORT, SPI EXPRESSLRS).
    /// For other protocols the feature is left untouched. Default true.
    #[serde(default = "default_true")]
    pub telemetry: bool,
    #[serde(default)]
    pub inverted: bool,
    #[serde(default)]
    pub half_duplex: bool,
    /// SPI EXPRESSLRS only: the 6-byte UID written as `expresslrs_uid`.
    /// None = leave the current UID (bind with `bind_rx`).
    /// The UI derives it from the binding phrase via `config_elrs_uid` and drops the
    /// phrase; the phrase is never part of the model, never stored and never logged.
    #[serde(default)]
    pub elrs_uid: Option<[u8; 6]>,
    /// SPI EXPRESSLRS only: regulatory domain. Must be one the board's chip
    /// supports (SX1280: ISM2400/CE2400; SX127x: the 433/868/915 MHz ones).
    #[serde(default)]
    pub elrs_domain: Option<ElrsDomain>,
}

fn default_true() -> bool {
    true
}

/// `expresslrs_domain` values (betaflight src/main/cli/settings.c lookupTableFreqDomain).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ElrsDomain {
    Au433,
    Au915,
    Eu433,
    Eu868,
    In866,
    Fcc915,
    Ism2400,
    Ce2400,
}

/// MVP config engine supports NONE, SERIAL and SPI (+ EXPRESSLRS only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The UI uses it to pick a table preset; the generator only writes `table`.
    pub region: Option<String>,
    /// Analog VTX table (`vtxtable ...`); null = keep the FC's table.
    #[serde(default)]
    pub table: Option<VtxTable>,
    /// Selected band / channel / power level, 1-based indexes into `table`
    /// (`vtx_band`, `vtx_channel`, `vtx_power`); null = keep.
    #[serde(default)]
    pub band: Option<u8>,
    #[serde(default)]
    pub channel: Option<u8>,
    #[serde(default)]
    pub power: Option<u8>,
}

/// Betaflight VTX table. Limits (drivers/vtx_table.h, same on 4.4 / 4.5 / 2025.12):
/// at most 8 bands, 8 channels per band, 8 power levels; band name <= 8 chars,
/// power label <= 3 chars; user frequencies <= 5999 MHz.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VtxTable {
    pub bands: Vec<VtxBand>,
    pub power_levels: Vec<VtxPowerLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VtxBand {
    /// e.g. "BOSCAM_A"
    pub name: String,
    /// One character, e.g. "A"
    pub letter: String,
    /// FACTORY (VTX uses its own frequencies) vs CUSTOM (FC sends `frequencies`).
    pub factory: bool,
    /// MHz per channel; 0 = channel disabled. Same length for every band.
    pub frequencies: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VtxPowerLevel {
    /// Protocol-specific value (SmartAudio 2.1: dBm; Tramp: mW; SmartAudio 1/2: index).
    pub value: u16,
    /// OSD label, up to 3 chars, e.g. "25", "200", "1W".
    pub label: String,
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
    /// Props-out ("reversed") motor direction (`yaw_motors_reversed`).
    /// Per-motor DShot spin-direction reversal is phase 2 (not in the MVP model).
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
    /// Name shown in OSD / Configurator (`set craft_name = ...`, all of 4.4 / 4.5 / 2025.12;
    /// the old `name` command no longer exists). 1-16 printable ASCII characters;
    /// `""` clears it (written as `-`, the firmware's empty value); null = keep.
    pub craft_name: Option<String>,
}

// ---------------------------------------------------------------- config engine

/// Firmware-version-specific CLI script produced by config::generate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliScript {
    /// Commands WITHOUT the surrounding `batch start`/`save`
    /// (config_apply adds those).
    pub commands: Vec<String>,
    /// Non-fatal issues to show the user (e.g. "UART3 also used by GPS").
    pub warnings: Vec<String>,
    /// Firmware dialect used: "4.4", "4.5" or "2025.12".
    pub dialect: String,
    /// Wizard-managed serial functions this script removes from ports that no
    /// longer need them (shown in the review step before applying).
    #[serde(default)]
    pub cleared: Vec<ClearedPort>,
}

/// One FC port whose managed functions are cleared by a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearedPort {
    /// e.g. "UART3"
    pub port: String,
    /// Functions removed. MSP appears only when an old MSP DisplayPort port had
    /// exactly MSP|VTX_MSP.
    pub functions: Vec<SerialFunction>,
    pub function_mask_before: u32,
    pub function_mask_after: u32,
}

/// Serial functions the config engine manages (others are never touched).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SerialFunction {
    RxSerial,
    Gps,
    EscSensor,
    VtxSmartaudio,
    VtxTramp,
    VtxMsp,
    Msp,
}

/// Emitted on event `config://progress` by config_apply / config_rollback / config_restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigProgress {
    pub stage: ConfigStage,
    /// 0.0..=1.0 within the stage (SENDING: lines sent / total)
    pub fraction: f32,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfigStage {
    Backup,
    Sending,
    Saving,
    Rebooting,
    Reconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    /// Stable id: "<boardUid>/<timestamp>" (timestamp = UTC YYYYMMDDTHHMMSSZ).
    #[serde(default)]
    pub id: String,
    /// Absolute path of the saved `diff all` text file:
    /// `<app_data_dir>/backups/<boardUid>/<timestamp>.txt` (kept forever).
    pub path: String,
    /// MCU unique id (hex, FcInfo.uid); "unknown" if unavailable.
    #[serde(default)]
    pub board_uid: String,
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
    /// Backup taken before writing (config_apply / config_restore always take one).
    #[serde(default)]
    pub backup: Option<Backup>,
    /// True once the FC rebooted after `save` and the CLI is reachable again.
    #[serde(default)]
    pub reconnected: bool,
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
    /// What was compared, e.g. "set motor_poles", "serial UART2", "feature TELEMETRY", "aux ARM".
    pub setting: String,
    pub expected: String,
    pub actual: String,
}

// ---------------------------------------------------------------- system

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverStatus {
    /// WinUSB bound to STM32/AT32 DFU (0483:DF11, 2E3C:DF11)
    pub dfu_winusb: bool,
    /// Vendor VCP / CP210x / CH340 drivers present
    pub vcp: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub os: String,
    pub cache_dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every example in the JSON schema must deserialize into the Rust mirror.
    #[test]
    fn schema_examples_deserialize() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/drone-model.schema.json")).unwrap();
        let examples = schema["examples"].as_array().unwrap();
        assert!(examples.len() >= 2);
        for ex in examples {
            let m: DroneModel = serde_json::from_value(ex.clone()).unwrap();
            assert_eq!(m.schema_version, DRONE_MODEL_SCHEMA_VERSION);
        }
    }

    #[test]
    fn receiver_telemetry_defaults_to_true() {
        let r: Receiver =
            serde_json::from_str(r#"{"kind":"SERIAL","protocol":"CRSF","port":"UART2"}"#).unwrap();
        assert!(r.telemetry);
        assert_eq!(r.elrs_domain, None);
    }

    /// The binding phrase is never part of the model: a stray `elrsBindingPhrase` from an
    /// older client is ignored on input and never serialized back out.
    #[test]
    fn binding_phrase_is_never_persisted() {
        let r: Receiver = serde_json::from_str(
            r#"{"kind":"SPI","protocol":"EXPRESSLRS","port":null,
                "elrsBindingPhrase":"secret phrase","elrsUid":[1,2,3,4,5,6]}"#,
        )
        .unwrap();
        assert_eq!(r.elrs_uid, Some([1, 2, 3, 4, 5, 6]));
        let out = serde_json::to_string(&r).unwrap();
        assert!(
            !out.contains("secret") && !out.to_lowercase().contains("phrase"),
            "{out}"
        );
    }
}
