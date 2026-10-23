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
    /// MSP_FC_VERSION, e.g. "4.5.2", "2025.12.1" or "2026.6.2"
    pub firmware_version: String,
    /// MSP_API_VERSION, e.g. "1.46"
    pub api_version: String,
    /// MSP_BOARD_INFO board name / Build API target, e.g. "SPEEDYBEEF405V4"
    pub board_name: String,
    /// Manufacturer id from MSP_BOARD_INFO, e.g. "SPBE"
    pub manufacturer_id: Option<String>,
    /// MCU type, e.g. "STM32F405"
    pub mcu: Option<String>,
    /// MCU unique id, identical to the CLI `mcu_id` output: the three MSP_UID (160) words
    /// (u32 little-endian) as `%08x%08x%08x` in order U_ID_0, U_ID_1, U_ID_2 — always 24
    /// lowercase hex chars. Used as the backup folder name. `None` only if MSP_UID fails.
    pub uid: Option<String>,
    /// Build options compiled in, as Build API defines (e.g. "USE_OSD_HD"), decoded from the
    /// MSP_BUILD_INFO option ids (API 1.46+, i.e. 4.5+). Ids with no known name are dropped.
    /// Empty on 4.4 (MSP_BUILD_INFO carries no options) or when unknown.
    pub build_options: Vec<String>,
    /// Whether this firmware build can drive an HD (MSP DisplayPort, 53x20 canvas) OSD, i.e.
    /// was built with `USE_OSD_HD`. `true`/`false` only on firm evidence (§4.1 "HD OSD
    /// support"); `null` when it cannot be determined (typically 4.4 with an OSD build).
    #[serde(default)]
    pub hd_osd_supported: Option<bool>,
    /// Serial ports exactly as the board reports them (MSP2_COMMON_SERIAL_CONFIG, falling
    /// back to MSP_CF_SERIAL_CONFIG). `identifier` is the raw firmware value (4.4/4.5:
    /// UART1=0; 2025.12+: UART1=51, and on 2026.6 UART11-15 = 61-65). Empty if the firmware answered neither command.
    /// Snapshot taken by `device_identify`; `config_read_ports` stays the source for config.
    #[serde(default)]
    pub serial_ports: Vec<FcSerialPort>,
    /// Onboard SPI receiver (e.g. ExpressLRS) detection. MSP alone cannot tell, so
    /// `rxSpi.present` is null after `device_identify` and filled by `device_probe_board`.
    #[serde(default)]
    pub rx_spi: RxSpiInfo,
}

/// Onboard SPI receiver information (`FcInfo.rxSpi`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RxSpiInfo {
    /// True when CLI `resource` shows `RX_SPI_CS` assigned to a pin (the board has an
    /// onboard SPI receiver); false when it is unassigned or the firmware has no RX_SPI.
    /// Null = not determined (no CLI probe yet).
    pub present: Option<bool>,
    /// `RX_SPI_EXPRESSLRS_RESET` and `RX_SPI_EXPRESSLRS_BUSY` are both assigned.
    pub expresslrs_pins: bool,
    /// `rx_spi_protocol` as printed by the firmware, e.g. "EXPRESSLRS" (CLI), or the name
    /// of the MSP_RX_CONFIG index as a hint when only MSP was read.
    pub protocol: Option<String>,
    /// `feature RX_SPI` state from MSP_FEATURE_CONFIG (null if not read).
    pub feature_enabled: Option<bool>,
    /// Raw assigned `RX_SPI*` resources, e.g. "RX_SPI_CS 1 A04".
    pub resources: Vec<String>,
}

/// A flight-controller-side serial port (UART pad), read from the FC.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FcSerialPort {
    /// Stable name used by the wizard and drone model: "UART1".."UART15",
    /// "LPUART1", "SOFTSERIAL1", "SOFTSERIAL2", "VCP".
    pub name: String,
    /// Firmware-specific numeric identifier used in the `serial` CLI command.
    /// NOTE: differs by firmware version (4.4/4.5: UART1=0; 2025.12+: UART1=51;
    /// 2026.6 adds UART11-15 = 61-65).
    /// Never hard-code - always take it from config_read_ports.
    pub identifier: i32,
    /// Current function bitmask (FUNCTION_* in betaflight src/main/io/serial.h).
    pub function_mask: u32,
    /// True for the port the app is connected through (USB VCP) - not offered to the user.
    pub is_connection_port: bool,
}

/// Motor-test limits. Always clamped by the backend: `maxThrottle` <= 1250,
/// `watchdogMs` 100..=2000. Defaults: 1150 / 500.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorTestLimits {
    pub max_throttle: u16,
    pub watchdog_ms: u32,
}

/// How the FC drives its motors (read at motor-test start).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorOutputConfig {
    /// `motor_pwm_protocol` name, e.g. "DSHOT600", "PWM", "DISABLED".
    pub protocol: String,
    /// DShot / ProShot.
    pub digital: bool,
    /// `feature 3D` (motor test is refused in 3D mode).
    pub feature3d: bool,
    pub mincommand: u16,
    pub motor_count: u8,
    /// Value meaning "motor stopped": 1000 for digital protocols, else `mincommand`.
    pub stop_value: u16,
}

/// Result of `device_motor_test_start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorTestSession {
    pub output_config: MotorOutputConfig,
    /// The limits actually in force (after clamping).
    pub limits: MotorTestLimits,
}

/// Emitted on `device://motor-test-stopped` whenever a running test ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorTestStopped {
    pub reason: MotorTestStopReason,
    /// `AppError` JSON when `reason` = ERROR.
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MotorTestStopReason {
    /// `device_motor_test_stop` or `device_disconnect`.
    Requested,
    /// No keepalive within `watchdogMs`.
    Watchdog,
    /// The FC became armed during the test.
    Armed,
    Disconnected,
    /// Window close/destroy or app exit (emergency stop in the Tauri run loop).
    AppClosing,
    Error,
}

/// Emitted on `device://rc-channels` at the stream rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RcChannels {
    pub seq: u64,
    /// Milliseconds since the stream started.
    pub timestamp_ms: u64,
    /// Raw MSP_RC values (usually 1000..2000), firmware channel order (AETR + AUX).
    pub channels: Vec<u16>,
}

/// Emitted on `device://rc-stopped` when the RC stream ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RcStopped {
    pub reason: RcStopReason,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RcStopReason {
    Requested,
    Disconnected,
    Error,
}

/// One sensor in `SensorStatus` (`device_get_sensors`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorState {
    /// Detected at boot and running (MSP_STATUS sensor bit). A sensor disabled in the
    /// config is never initialised, so it is never present.
    pub present: bool,
    /// Not disabled in the config (`acc_hardware` / `baro_hardware` / `mag_hardware`
    /// != NONE). Always true for the gyro (it cannot be disabled).
    pub enabled: bool,
    /// The configured hardware setting as the CLI spells it ("AUTO", "NONE", "BMI270", ...).
    /// `null` for the gyro.
    pub configured_hardware: Option<String>,
    /// The chip the FC detected (MSP2_SENSOR_CONFIG_ACTIVE, Betaflight 4.5+), e.g. "BMI270".
    /// `null` on 4.4, when nothing was detected, or when the build has no driver for it.
    pub detected_hardware: Option<String>,
}

/// Board / sensor alignment as currently stored on the FC (read-only; writes belong to
/// the config engine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorAlignment {
    /// `align_board_roll` / `_pitch` / `_yaw` in whole degrees (MSP_BOARD_ALIGNMENT_CONFIG).
    pub board_roll_deg: i16,
    pub board_pitch_deg: i16,
    pub board_yaw_deg: i16,
    /// `gyro_1_sensor_align` of the active gyro (MSP_SENSOR_ALIGNMENT): "DEFAULT", "CW0",
    /// "CW90", "CW180", "CW270", "CW0FLIP", "CW90FLIP", "CW180FLIP", "CW270FLIP", "CUSTOM".
    pub gyro_alignment: String,
}

/// Result of `device_get_sensors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorStatus {
    pub gyro: SensorState,
    pub acc: SensorState,
    pub baro: SensorState,
    pub mag: SensorState,
    /// `false` while Betaflight reports "accelerometer needs calibration"
    /// (MSP_BOARD_INFO configuration problem bit 0); `null` if unknown or no acc.
    pub acc_calibrated: Option<bool>,
    /// `null` if the FC did not answer the alignment requests.
    pub alignment: Option<SensorAlignment>,
}

/// How `device_acc_calibrate` saw the calibration finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AccCalibrationCompletion {
    /// MSP_STATUS_EX arming-disable flag CALIBRATING was seen set and then clear.
    CalibratingFlag,
    /// The flag was not observable; finished after the fixed wait (2 s, as Betaflight
    /// Configurator does).
    FixedWait,
}

/// Result of `device_acc_calibrate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccCalibrationResult {
    /// From sending MSP_ACC_CALIBRATION to completion.
    pub duration_ms: u32,
    pub completion: AccCalibrationCompletion,
    /// Largest gyro rate seen (deg/s, any axis) while calibrating.
    pub max_gyro_rate_dps: u16,
}

/// Emitted on `device://attitude` at the stream rate; also the result of
/// `device_get_attitude` (with `seq` = 0, `timestampMs` = 0). Degrees, Betaflight signs:
/// roll + = right side down, pitch + = nose down, yaw = heading (+ = nose right).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attitude {
    pub seq: u64,
    /// Milliseconds since the stream started.
    pub timestamp_ms: u64,
    /// Degrees, -180..180 (0.1° resolution). Signs are Betaflight's MSP_ATTITUDE values
    /// unchanged; Configurator's 3D model rotates by (x, y, z) = (-pitch, -yaw, -roll).
    pub roll: f32,
    /// Degrees, -90..90 (0.1° resolution).
    pub pitch: f32,
    /// Heading in whole degrees, 0..359.
    pub yaw: f32,
}

/// Emitted on `device://attitude-stopped` when the attitude stream ends (same reasons as
/// the RC stream).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttitudeStopped {
    pub reason: RcStopReason,
    pub error: Option<serde_json::Value>,
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
    /// e.g. "4.5.2", "2025.12.1", "2026.6.2"
    pub release: String,
    pub prerelease: bool,
}

/// What to build/fetch. Options are Build API option values (e.g. "USE_GPS"); see
/// ARCHITECTURE §6 "Build options" for how they are resolved per release.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareRequest {
    pub target: String,
    pub release: String,
    /// The board's current build options (`FcInfo.buildOptions`). Filtered to what the
    /// release offers; anything else is dropped and reported in `FirmwareImage.droppedOptions`.
    /// Empty (4.4 boards, no board) = the release's default options.
    pub options: Vec<String>,
    /// The wizard's choices (`flasher_required_options`): always added, never dropped. If the
    /// release does not offer one, the request fails with `REQUIRED_OPTION_UNSUPPORTED`.
    #[serde(default)]
    pub required_options: Vec<String>,
    /// If true, never touch the network; fail if not cached.
    pub offline_only: bool,
}

/// The slice of the wizard's answers that needs firmware build options
/// (`flasher_required_options`). Field names and enums are `DroneModel`'s, so the UI can
/// pass the model itself (other fields are ignored).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareRequirements {
    #[serde(default)]
    pub receiver: Option<ReceiverRequirement>,
    #[serde(default)]
    pub osd: Option<OsdRequirement>,
}

/// `DroneModel.receiver` (kind + protocol only).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiverRequirement {
    pub kind: ReceiverKind,
    #[serde(default)]
    pub protocol: Option<RxProtocol>,
}

/// `DroneModel.osd` (video system only).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsdRequirement {
    #[serde(default)]
    pub video_system: Option<OsdVideoSystem>,
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
    /// Board options (`request.options`) left out because the release does not offer them
    /// (e.g. USE_WING when going from 2025.12 to 4.5). Required options are never dropped.
    #[serde(default)]
    pub dropped_options: Vec<String>,
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
    /// Air mode: ALWAYS = `feature AIRMODE`; SWITCH = `feature -AIRMODE` plus an "AIR MODE"
    /// entry in `switches` (required); OFF = `feature -AIRMODE` and no AIR MODE range.
    /// null = keep (feature and ranges untouched). Same mechanism on every supported firmware line.
    #[serde(default)]
    pub airmode: Option<Airmode>,
    /// Optional modes (PREARM, FLIP OVER AFTER CRASH, AIR MODE) whose ranges on the FC are
    /// removed. Optional modes not listed here and not in `switches` are kept as they are.
    #[serde(default)]
    pub remove_modes: Vec<String>,
    /// Beeper conditions (`beeper` / `beacon` CLI commands); null = keep.
    #[serde(default)]
    pub beepers: Option<Beepers>,
    /// Sensor enables (acc/baro/mag_hardware); null = keep. The gyro is always on.
    #[serde(default)]
    pub sensors: Option<Sensors>,
    /// align_board_roll / _pitch / _yaw; null = keep.
    #[serde(default)]
    pub board_alignment: Option<BoardAlignment>,
    /// OSD layout; null = keep the FC's OSD untouched.
    #[serde(default)]
    pub osd: Option<Osd>,
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

/// Betaflight VTX table. Limits (drivers/vtx_table.h, same on every supported firmware line):
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
    /// UBLOX only: also track Galileo (`gps_ublox_use_galileo`). GPS is always on;
    /// GLONASS / BeiDou are not configurable on any supported firmware line. null = keep.
    #[serde(default)]
    pub galileo: Option<bool>,
    /// SBAS augmentation (`gps_sbas_mode`); null = keep.
    #[serde(default)]
    pub sbas_mode: Option<SbasMode>,
}

/// `gps_sbas_mode` values (same table on every supported firmware line).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SbasMode {
    Auto,
    Egnos,
    Waas,
    Msas,
    Gagan,
    None,
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
    /// Motor idle in 0.01 % (550 = 5.5 %), 0..=2000. Written as `dshot_idle_value` on
    /// 4.4 / 4.5 (exists only with DShot) and `motor_idle` on 2025.12+; null = keep.
    #[serde(default)]
    pub idle: Option<u16>,
    /// Motor remap (`motor_output_reordering`, every supported firmware line): entry i is the
    /// 0-based output that drives motor i+1. Must be a permutation of 0..n where n is
    /// the motor count (4 for a quad); null = keep.
    #[serde(default)]
    pub motor_order: Option<Vec<u8>>,
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
    /// Name shown in OSD / Configurator (`set craft_name = ...`, every supported firmware line;
    /// the old `name` command no longer exists). See [`craft_name_valid`];
    /// `""` clears it (written as `-`, the firmware's empty value); null = keep.
    pub craft_name: Option<String>,
}

/// `extras.craftName` pattern in schemas/drone-model.schema.json (kept identical; a test
/// compares them). Equivalent to [`craft_name_valid`].
pub const CRAFT_NAME_PATTERN: &str = r"^(?:[\x21\x22\x24-\x2C\x2E-\x3E\x40-\x7E]|[\x21\x22\x24-\x3E\x40-\x7E][\x20-\x22\x24-\x3E\x40-\x7E]{0,14}[\x21\x22\x24-\x3E\x40-\x7E])?$";

/// What every supported Betaflight line can store through `set craft_name = <name>`
/// (craft_name is MODE_STRING 1..MAX_NAME_LENGTH = 16, cli/settings.c; parsing in cli.c):
/// - processCharacter only buffers 0x20..=0x7E;
/// - `#` starts a comment (the rest of the line is cut), `?` is tab-completion in the
///   interactive CLI that MSP `#` entry opens (`cliEnter(port, true)`);
/// - trailing spaces are stripped from the line, leading ones by `skipSpace`;
/// - exactly `-` is the empty marker (strncmp against `emptyName`).
///
/// `""` (clear) is valid here; the generator writes it as `-`.
pub fn craft_name_valid(name: &str) -> bool {
    let printable = |c: char| matches!(c, ' '..='~') && c != '#' && c != '?';
    name.is_empty()
        || (name.len() <= MAX_CRAFT_NAME_LENGTH
            && name.chars().all(printable)
            && !name.starts_with(' ')
            && !name.ends_with(' ')
            && name != "-")
}

/// pg/pilot.h MAX_NAME_LENGTH.
pub const MAX_CRAFT_NAME_LENGTH: usize = 16;

/// Highest `vtx.power` the CLI accepts: `vtx_power` is 1-based (0 = unknown) with range
/// 0..VTX_TABLE_MAX_POWER_LEVELS-1 = 0..7 on every supported firmware line (cli/settings.c), so a
/// table's 8th power level cannot be selected via the CLI. Band / channel are 1..=8.
pub const VTX_POWER_MAX: u8 = 7;
pub const VTX_BAND_MAX: u8 = 8;
pub const VTX_CHANNEL_MAX: u8 = 8;

// ---------------------------------------------------------------- drone model, batch 2

/// `DroneModel.airmode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Airmode {
    Always,
    Switch,
    Off,
}

/// Stable beeper condition names = the 4.4 / 4.5 CLI names (`beeper list`). Config maps
/// them per firmware: CRASH_FLIP is `CRASHFLIP` on 2025.12+; RC_SMOOTHING_INIT_FAIL does
/// not exist on 2025.12+ (a warning, nothing written).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BeeperName {
    GyroCalibrated,
    RxLost,
    RxLostLanding,
    Disarming,
    Arming,
    ArmingGpsFix,
    ArmingGpsNoFix,
    BatCritLow,
    BatLow,
    GpsStatus,
    RxSet,
    AccCalibration,
    AccCalibrationFail,
    ReadyBeep,
    MultiBeeps,
    DisarmRepeat,
    Armed,
    SystemInit,
    OnUsb,
    BlackboxErase,
    CrashFlip,
    CamConnectionOpen,
    CamConnectionClose,
    RcSmoothingInitFail,
}

/// DShot beacon conditions (`beacon`): the only two the firmware allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DshotBeaconCondition {
    RxLost,
    RxSet,
}

/// Full beeper state: every condition the firmware has is written, `beeper -X` for the
/// listed ones and `beeper X` for the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Beepers {
    pub disabled: Vec<BeeperName>,
    /// `beacon -X` for listed, `beacon X` for the other; null = keep.
    #[serde(default)]
    pub dshot_beacon_disabled: Option<Vec<DshotBeaconCondition>>,
}

/// true = `*_hardware = AUTO` (a specific chip already set on the FC is kept),
/// false = `NONE`; null = keep. Gyro is always on.
/// Device's `SensorState.configured_hardware` reads the same setting (`NONE` = false /
/// disabled, anything else = true).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sensors {
    #[serde(default)]
    pub acc: Option<bool>,
    #[serde(default)]
    pub baro: Option<bool>,
    #[serde(default)]
    pub mag: Option<bool>,
}

/// Whole degrees, the firmware range -180..=360 on every supported firmware line (cli/settings.c);
/// the same values MSP_BOARD_ALIGNMENT_CONFIG reports (int16 each).
/// `roll` / `pitch` / `yaw` = device `SensorAlignment.board_roll_deg` / `_pitch_deg` / `_yaw_deg`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardAlignment {
    pub roll: i16,
    pub pitch: i16,
    pub yaw: i16,
}
pub const BOARD_ALIGNMENT_MIN: i16 = -180;
pub const BOARD_ALIGNMENT_MAX: i16 = 360;

/// `vcd_video_system` (same table on every supported firmware line). The OSD grid is
/// PAL 30x16, NTSC 30x13, AUTO 30x13 (fits either camera), HD 53x20.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OsdVideoSystem {
    Auto,
    Pal,
    Ntsc,
    Hd,
}

impl OsdVideoSystem {
    /// (columns, rows) of the grid elements must fit in.
    pub fn grid(self) -> (u8, u8) {
        match self {
            OsdVideoSystem::Pal => (30, 16),
            OsdVideoSystem::Ntsc | OsdVideoSystem::Auto => (30, 13),
            OsdVideoSystem::Hd => (53, 20),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Osd {
    /// null = keep the FC's video system (the grid then comes from the FC's settings).
    #[serde(default)]
    pub video_system: Option<OsdVideoSystem>,
    /// Elements not listed keep the FC's state.
    #[serde(default)]
    pub elements: Vec<OsdElement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsdElement {
    /// Stable name = Betaflight's osd_items_e name without `OSD_` (e.g. MAIN_BATT_VOLTAGE,
    /// written as `osd_vbat_pos`). Unknown for the firmware -> CONFIG.
    pub name: String,
    pub enabled: bool,
    /// Column / row; null = config's default layout position for the video system.
    #[serde(default)]
    pub x: Option<u8>,
    #[serde(default)]
    pub y: Option<u8>,
}

/// `config_osd_default_layout` result: config's default positions (the single source
/// of truth for the UI preview and the generator).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsdLayout {
    pub video_system: OsdVideoSystem,
    pub cols: u8,
    pub rows: u8,
    /// Every element the firmware line has, with config's default x/y and default visibility.
    pub elements: Vec<OsdLayoutElement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsdLayoutElement {
    pub name: String,
    pub x: u8,
    pub y: u8,
    pub enabled: bool,
}

/// `config_firmware_catalog` result: what a firmware line offers, so the UI does not
/// hard-code per-version lists. Read-only, no FC I/O; a particular board's build may lack
/// some of it (config checks the FC's `dump all` at plan time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareCatalog {
    /// Beeper conditions the line has (stable names; 2025.12+ lacks RC_SMOOTHING_INIT_FAIL).
    pub beepers: Vec<BeeperName>,
    /// DShot beacon conditions (RX_LOST, RX_SET on every line).
    pub beacon_events: Vec<DshotBeaconCondition>,
    /// OSD element stable names, in `cli/settings.c` order.
    pub osd_elements: Vec<String>,
    /// Stable mode names accepted in `switches` / `removeModes` (canonical names).
    pub modes: Vec<String>,
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
    /// Firmware dialect used: "4.4", "4.5", "2025.12" or "2026.6" (one per
    /// `config::firmware::SUPPORTED_LINES` entry).
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

/// Emitted on event `config://progress` by config_apply / config_rollback / config_restore /
/// config_restore_after_flash.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigProgress {
    pub stage: ConfigStage,
    /// 0.0..=1.0 within the stage (SENDING / RESTORING: lines sent / total)
    pub fraction: f32,
    /// Never contains a backup line (they can hold secrets); RESTORING says "line n/total".
    pub message: Option<String>,
    /// RESTORING only: 1-based number of the backup line being replayed (0 before the first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// RESTORING only: number of backup lines (the `diff all` body, comments included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConfigStage {
    Backup,
    Sending,
    /// config_restore_after_flash: replaying the backup (progress carries `line` / `total`).
    Restoring,
    Saving,
    Rebooting,
    Reconnected,
}

/// Result of `config_restore_after_flash` (ARCHITECTURE.md §5.2). Every non-empty line of
/// the backup's `diff all` body is counted exactly once: in `applied`, `failed` or `skipped`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    /// Lines the FC accepted (after dialect mapping), `defaults nosave` included.
    pub applied: usize,
    /// Lines the FC rejected; the restore continued past them.
    pub failed: Vec<RestoreFailure>,
    /// Lines deliberately not sent, each with a reason.
    pub skipped: Vec<RestoreSkip>,
    /// Mapping notes (renames, battery-profile placement, UID policy), read-back differences.
    pub warnings: Vec<String>,
    /// True once `save` was accepted (also when some lines failed: intended on this path).
    pub saved: bool,
    /// The backup's firmware line differs from the flashed one (dialect mapping was used).
    pub line_changed: bool,
    /// Full firmware version of the backup, e.g. "2025.12.5".
    pub from_version: String,
    /// Full firmware version now on the board, e.g. "2026.6.2".
    pub to_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreFailure {
    /// The backup line (secret values masked).
    pub line: String,
    /// The FC's error, plus the command actually sent when the line was mapped.
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSkip {
    /// The backup line (secret values masked).
    pub line: String,
    pub reason: RestoreSkipReason,
    /// Human-readable explanation (e.g. "removed in 2025.12").
    pub detail: String,
}

/// Why a backup line was not sent (ARCHITECTURE.md §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RestoreSkipReason {
    /// A comment, including the `# version` banner.
    Comment,
    /// `diff all` / `batch start` / `batch end` / `save`: the restore sends its own.
    SessionControl,
    /// board_name, manufacturer_id, mcu_id, signature: identity of the hardware, never replayed.
    BoardIdentity,
    /// resource / timer / dma when the firmware line changed (pin maps differ per line).
    HardwareLineChanged,
    /// The setting, feature, beeper, value or command does not exist on the target line.
    RemovedOnTarget,
    /// The setting exists on the target line but not in this firmware build.
    NotInBuild,
    /// Same name, different meaning on the target line (D-term across 4.5 ↔ 2025.12).
    SemanticsChanged,
    /// An aux mode or adjustment function with no equivalent on the target line.
    NoEquivalentMode,
    /// A serial port the target firmware / board does not have.
    PortNotAvailable,
    /// A 2026.6 battery profile other than the selected one, restored onto a line without
    /// battery profiles.
    OtherBatteryProfile,
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
    /// MCU unique id (FcInfo.uid: 24-char `mcu_id` hex); "unknown" if unavailable.
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
    /// DFU only: true when `action = INSTALL_DFU_DRIVER` and another driver
    /// (`service`, e.g. ST's DfuSe "STTub30" or libusb0) is currently bound, which
    /// installing WinUSB will REPLACE (STM32CubeProgrammer/DfuSe stop seeing the
    /// device). The UI must warn and get confirmation first. False when no driver
    /// is bound.
    #[serde(default)]
    pub replaces_driver: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_report_and_progress_wire_format() {
        let r = RestoreReport {
            applied: 3,
            failed: vec![RestoreFailure {
                line: "set angle_limit = 90".into(),
                error: "###ERROR IN set: INVALID VALUE###".into(),
            }],
            skipped: vec![RestoreSkip {
                line: "board_name SPEEDYBEEF405V4".into(),
                reason: RestoreSkipReason::BoardIdentity,
                detail: "hardware identity".into(),
            }],
            warnings: vec![],
            saved: true,
            line_changed: true,
            from_version: "2025.12.5".into(),
            to_version: "2026.6.2".into(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["lineChanged"], true);
        assert_eq!(v["fromVersion"], "2025.12.5");
        assert_eq!(v["toVersion"], "2026.6.2");
        assert_eq!(v["failed"][0]["error"], "###ERROR IN set: INVALID VALUE###");
        assert_eq!(v["skipped"][0]["reason"], "BOARD_IDENTITY");
        let p = ConfigProgress {
            stage: ConfigStage::Restoring,
            fraction: 0.5,
            message: Some("line 5/10".into()),
            line: Some(5),
            total: Some(10),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["stage"], "RESTORING");
        assert_eq!(
            (v["line"].as_u64(), v["total"].as_u64()),
            (Some(5), Some(10))
        );
        let v = serde_json::to_value(ConfigProgress {
            stage: ConfigStage::Saving,
            fraction: 0.0,
            message: None,
            line: None,
            total: None,
        })
        .unwrap();
        assert!(v.get("line").is_none() && v.get("total").is_none());
    }

    /// The craft-name rule is one rule: the schema's pattern is CRAFT_NAME_PATTERN, and
    /// `craft_name_valid` agrees with every shared case (tests/model-limits.test.ts runs the
    /// same cases through the schema pattern in JS).
    #[test]
    fn craft_name_rule_matches_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/drone-model.schema.json")).unwrap();
        let extras = &schema["properties"]["extras"]["properties"]["craftName"];
        assert_eq!(extras["pattern"].as_str(), Some(CRAFT_NAME_PATTERN));
        assert_eq!(
            extras["maxLength"].as_u64(),
            Some(MAX_CRAFT_NAME_LENGTH as u64)
        );
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/fixtures/craft-name-cases.json"))
                .unwrap();
        for (key, want) in [("valid", true), ("invalid", false)] {
            for c in cases[key].as_array().unwrap() {
                let c = c.as_str().unwrap();
                assert_eq!(craft_name_valid(c), want, "{c:?}");
            }
        }
        // Exhaustive over single chars: only 0x21..=0x7E minus # ? - are valid alone.
        for b in 0u8..=255 {
            let c = (b as char).to_string();
            let want = (0x21..=0x7e).contains(&b) && !b"#?-".contains(&b);
            assert_eq!(craft_name_valid(&c), want, "{b:#x}");
        }
    }

    /// vtx band/channel/power maxima in the schema = firmware (drivers/vtx_table.h).
    #[test]
    fn vtx_limits_match_schema_and_firmware() {
        use crate::config::vtx_table;
        assert_eq!(usize::from(VTX_POWER_MAX), vtx_table::MAX_POWER_LEVELS - 1);
        assert_eq!(usize::from(VTX_BAND_MAX), vtx_table::MAX_BANDS);
        assert_eq!(usize::from(VTX_CHANNEL_MAX), vtx_table::MAX_CHANNELS);
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/drone-model.schema.json")).unwrap();
        let vtx = &schema["properties"]["vtx"]["properties"];
        for (k, max) in [
            ("band", VTX_BAND_MAX),
            ("channel", VTX_CHANNEL_MAX),
            ("power", VTX_POWER_MAX),
        ] {
            assert_eq!(vtx[k]["minimum"].as_u64(), Some(1), "{k}");
            assert_eq!(vtx[k]["maximum"].as_u64(), Some(u64::from(max)), "{k}");
        }
    }

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

    /// Batch-2 enums and limits agree with the schema; absent fields mean keep.
    #[test]
    fn batch2_shape_matches_schema() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/drone-model.schema.json")).unwrap();
        let names: Vec<BeeperName> =
            serde_json::from_value(schema["$defs"]["beeperName"]["enum"].clone()).unwrap();
        assert_eq!(names.len(), 24);
        let roll = &schema["properties"]["boardAlignment"]["properties"]["roll"];
        assert_eq!(roll["minimum"], BOARD_ALIGNMENT_MIN);
        assert_eq!(roll["maximum"], BOARD_ALIGNMENT_MAX);
        assert_eq!(OsdVideoSystem::Pal.grid(), (30, 16));
        assert_eq!(OsdVideoSystem::Hd.grid(), (53, 20));
        let ex = &schema["examples"][0];
        let m: DroneModel = serde_json::from_value(ex.clone()).unwrap();
        assert_eq!(m.airmode, Some(Airmode::Switch));
        assert_eq!(m.beepers.unwrap().disabled, vec![BeeperName::OnUsb]);
        let mut bare = ex.clone();
        for k in [
            "airmode",
            "removeModes",
            "beepers",
            "sensors",
            "boardAlignment",
            "osd",
        ] {
            bare.as_object_mut().unwrap().remove(k);
        }
        let m: DroneModel = serde_json::from_value(bare).unwrap();
        assert!(m.airmode.is_none() && m.remove_modes.is_empty() && m.osd.is_none());
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
