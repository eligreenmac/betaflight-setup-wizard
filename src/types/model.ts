// Mirror of src-tauri/src/model.rs and schemas/drone-model.schema.json.
// Change only in a PR labelled `shared` that updates all three together.

export const DRONE_MODEL_SCHEMA_VERSION = 1;

// ---- device
export type HostPortKind = "FC_VCP" | "USB_UART_BRIDGE" | "UNKNOWN";
export interface HostPort {
  path: string;
  vid: number | null;
  pid: number | null;
  manufacturer: string | null;
  product: string | null;
  kind: HostPortKind;
}
export interface FcInfo {
  fcVariant: string;
  firmwareVersion: string;
  apiVersion: string;
  boardName: string;
  manufacturerId: string | null;
  mcu: string | null;
  /** MCU unique id exactly as CLI `mcu_id` prints it: 24 lowercase hex chars
   * (`%08x%08x%08x` of U_ID_0..2). Backup folder name. null only if MSP_UID failed. */
  uid: string | null;
  buildOptions: string[];
  /** Serial ports exactly as the board reports them (raw identifiers). Empty if unknown. */
  serialPorts: FcSerialPort[];
  rxSpi: RxSpiInfo;
}
/** Onboard SPI receiver detection. `present` is null until device_probe_board ran. */
export interface RxSpiInfo {
  present: boolean | null;
  expresslrsPins: boolean;
  protocol: string | null;
  featureEnabled: boolean | null;
  resources: string[];
}
export interface FcSerialPort {
  /** "UART1".."UART15", "LPUART1", "SOFTSERIAL1", "SOFTSERIAL2", "VCP" */
  name: string;
  /** firmware-specific id (4.4/4.5: UART1=0; 2025.12+: UART1=51) */
  identifier: number;
  functionMask: number;
  isConnectionPort: boolean;
}

// ---- flasher
export interface BoardTarget { target: string; manufacturer: string | null; mcu: string | null }
export interface FirmwareRelease { release: string; prerelease: boolean }
export interface FirmwareRequest { target: string; release: string; options: string[]; offlineOnly: boolean }
export type FirmwareFormat = "HEX" | "BIN" | "UF2";
export interface FirmwareImage {
  request: FirmwareRequest;
  path: string;
  format: FirmwareFormat;
  sha256: string;
  buildKey: string | null;
  fromCache: boolean;
}
export interface DfuDevice {
  vid: number;
  pid: number;
  serial: string | null;
  mcuFamily: string | null;
  flashDescriptor: string | null;
  driverOk: boolean;
}
export interface FlashOptions { fullErase: boolean; manifest: boolean }
export type FlashStage =
  | "DOWNLOADING" | "ENTERING_BOOTLOADER" | "WAITING_FOR_DEVICE" | "ERASING"
  | "WRITING" | "VERIFYING" | "REBOOTING" | "DONE" | "FAILED";
export interface FlashProgress { stage: FlashStage; fraction: number; message: string | null }

// ---- drone model (wizard answers)
export type ReceiverKind = "NONE" | "SERIAL" | "SPI" | "PPM" | "MSP";
export type RxProtocol =
  | "CRSF" | "GHST" | "SBUS" | "FPORT" | "IBUS" | "SRXL2" | "SPEK1024" | "SPEK2048" | "SUMD"
  | "EXPRESSLRS" | "FRSKY_D" | "FRSKY_X" | "FRSKY_X_LBT";
export type VtxKind = "NONE" | "ANALOG" | "DIGITAL";
export type VtxProtocol = "SMARTAUDIO" | "TRAMP" | "MSP";
export type DigitalSystem = "DJI" | "WALKSNAIL" | "HDZERO" | "OTHER";
export type GpsProvider = "UBLOX" | "NMEA";
export type EscProtocol = "DSHOT150" | "DSHOT300" | "DSHOT600" | "ONESHOT125" | "MULTISHOT" | "PWM";

export interface DroneModel {
  schemaVersion: number;
  fc: { boardName: string; firmwareVersion: string; apiVersion: string };
  receiver: {
    kind: ReceiverKind;
    protocol: RxProtocol | null;
    port: string | null;
    /** CRSF/FPORT/SPI EXPRESSLRS: true -> `feature TELEMETRY` (default true). */
    telemetry: boolean;
    inverted: boolean;
    halfDuplex: boolean;
    /**
     * SPI EXPRESSLRS only: 6-byte UID (from `api.config.elrsUid(phrase)`); null = keep current UID.
     * Never put the binding phrase in the model, local storage or logs: hash it, then drop it.
     */
    elrsUid?: number[] | null;
    /** SPI EXPRESSLRS only; must match the board's chip (SX1280: ISM2400/CE2400). */
    elrsDomain?: ElrsDomain | null;
  };
  vtx: {
    kind: VtxKind;
    protocol: VtxProtocol | null;
    port: string | null;
    digitalSystem: DigitalSystem | null;
    region: string | null;
    /** Analog VTX table (`vtxtable`); null = keep the FC's table. */
    table?: VtxTable | null;
    /** 1-based band / channel / power level into `table`; null = keep.
     *  band, channel: 1..VTX_BAND_MAX / VTX_CHANNEL_MAX (8); power: 1..VTX_POWER_MAX (7). */
    band?: number | null;
    channel?: number | null;
    power?: number | null;
  };
  gps: {
    enabled: boolean;
    port: string | null;
    provider: GpsProvider | null;
    /** UBLOX only: gps_ublox_use_galileo; null = keep. */
    galileo?: boolean | null;
    sbasMode?: SbasMode | null;
  };
  esc: {
    protocol: EscProtocol;
    bidirectionalDshot: boolean;
    motorPoles: number | null;
    telemetryPort: string | null;
    propsOut: boolean;
    /** Motor idle in 0.01 % (550 = 5.5 %), 0..2000; null = keep. */
    idle?: number | null;
    /** motor_output_reordering: item i = 0-based output for motor i+1 (permutation); null = keep. */
    motorOrder?: number[] | null;
  };
  switches: { mode: string; auxChannel: number; rangeMin: number; rangeMax: number }[];
  extras: { beeper?: boolean | null; ledStrip?: boolean | null; craftName?: string | null /* CRAFT_NAME_PATTERN; "" clears; null keeps */ };
  /** ALWAYS = feature AIRMODE; SWITCH = needs an "AIR MODE" entry in `switches`; OFF; null = keep. */
  airmode?: Airmode | null;
  /** Optional modes whose FC ranges are removed; optional modes neither here nor in `switches` are kept. */
  removeModes?: OptionalMode[];
  /** null = keep. */
  beepers?: Beepers | null;
  /** true = AUTO (a specific chip on the FC is kept), false = NONE, null = keep. Gyro is always on. */
  sensors?: { acc?: boolean | null; baro?: boolean | null; mag?: boolean | null } | null;
  /** Whole degrees, -180..360 (= what device_sensors() reports); null = keep. */
  boardAlignment?: BoardAlignment | null;
  /** null = keep the FC's OSD. */
  osd?: Osd | null;
}

export type Airmode = "ALWAYS" | "SWITCH" | "OFF";
/** Stable names in `switches[].mode` for the optional modes (config::modes canonical names). */
export type OptionalMode = "PREARM" | "FLIP OVER AFTER CRASH" | "AIR MODE";

/** 4.4/4.5 CLI names. 2025.12: CRASH_FLIP is written as CRASHFLIP; RC_SMOOTHING_INIT_FAIL doesn't exist (warning). */
export const BEEPER_NAMES = [
  "GYRO_CALIBRATED", "RX_LOST", "RX_LOST_LANDING", "DISARMING", "ARMING", "ARMING_GPS_FIX",
  "ARMING_GPS_NO_FIX", "BAT_CRIT_LOW", "BAT_LOW", "GPS_STATUS", "RX_SET", "ACC_CALIBRATION",
  "ACC_CALIBRATION_FAIL", "READY_BEEP", "MULTI_BEEPS", "DISARM_REPEAT", "ARMED", "SYSTEM_INIT",
  "ON_USB", "BLACKBOX_ERASE", "CRASH_FLIP", "CAM_CONNECTION_OPEN", "CAM_CONNECTION_CLOSE",
  "RC_SMOOTHING_INIT_FAIL",
] as const;
export type BeeperName = (typeof BEEPER_NAMES)[number];
export type DshotBeaconCondition = "RX_LOST" | "RX_SET";
/** Full state: `beeper -X` for listed, `beeper X` for every other condition the firmware has. */
export interface Beepers {
  disabled: BeeperName[];
  /** `beacon -X` for listed, `beacon X` for the other; null = keep. */
  dshotBeaconDisabled?: DshotBeaconCondition[] | null;
}

/** Same values as device_sensors() `SensorAlignment` boardRollDeg / boardPitchDeg / boardYawDeg (signed whole degrees). */
export interface BoardAlignment { roll: number; pitch: number; yaw: number }

/** vcd_video_system. Grid: PAL 30x16, NTSC 30x13, AUTO 30x13, HD 53x20. */
export type OsdVideoSystem = "AUTO" | "PAL" | "NTSC" | "HD";
/** Betaflight osd_items_e name without OSD_, e.g. MAIN_BATT_VOLTAGE; per-firmware list from config_osd_default_layout. */
export type OsdElementName = string;
export interface OsdElement {
  name: OsdElementName;
  /** Visible in all 3 OSD profiles, or none. */
  enabled: boolean;
  /** null/absent = default layout position. Must fit the grid (CONFIG error otherwise). */
  x?: number | null;
  y?: number | null;
}
export interface Osd {
  /** null = keep the FC's video system (grid from the FC). */
  videoSystem?: OsdVideoSystem | null;
  /** Elements not listed keep the FC's state. */
  elements: OsdElement[];
}
/** config_osd_default_layout(videoSystem, firmwareVersion) */
export interface OsdLayout {
  videoSystem: OsdVideoSystem;
  cols: number;
  rows: number;
  elements: { name: OsdElementName; x: number; y: number; enabled: boolean }[];
}

/**
 * extras.craftName rule (= schemas/drone-model.schema.json and model::craft_name_valid in Rust):
 * 1-16 chars of printable ASCII except `#` and `?`, no leading/trailing space, not exactly "-";
 * "" clears the name. Betaflight's CLI would cut, strip or misread anything else.
 */
export const CRAFT_NAME_PATTERN = /^(?:[\x21\x22\x24-\x2C\x2E-\x3E\x40-\x7E]|[\x21\x22\x24-\x3E\x40-\x7E][\x20-\x22\x24-\x3E\x40-\x7E]{0,14}[\x21\x22\x24-\x3E\x40-\x7E])?$/;
export const CRAFT_NAME_MAX_LENGTH = 16;
/** vtx_power is 1-based with CLI range 0..7: a table's 8th power level cannot be selected. */
export const VTX_POWER_MAX = 7;
export const VTX_BAND_MAX = 8;
export const VTX_CHANNEL_MAX = 8;

export type SbasMode = "AUTO" | "EGNOS" | "WAAS" | "MSAS" | "GAGAN" | "NONE";
/** Limits (4.4/4.5/2025.12): <= 8 bands, 8 channels, 8 power levels (only 1..VTX_POWER_MAX selectable); name <= 8, label <= 3 chars. */
export interface VtxTable {
  bands: { name: string; letter: string; factory: boolean; frequencies: number[] }[];
  powerLevels: { value: number; label: string }[];
}

// ---- config engine
export type ElrsDomain = "AU433" | "AU915" | "EU433" | "EU868" | "IN866" | "FCC915" | "ISM2400" | "CE2400";
export type SerialFunction = "RX_SERIAL" | "GPS" | "ESC_SENSOR" | "VTX_SMARTAUDIO" | "VTX_TRAMP" | "VTX_MSP" | "MSP";
export interface ClearedPort {
  port: string;
  functions: SerialFunction[];
  functionMaskBefore: number;
  functionMaskAfter: number;
}
export interface CliScript { commands: string[]; warnings: string[]; dialect: string; cleared: ClearedPort[] }
export interface Backup {
  id: string;
  path: string;
  /** FcInfo.uid (24-char mcu_id hex) or "unknown". */
  boardUid: string;
  boardName: string;
  firmwareVersion: string;
  createdAt: string;
}
export interface CliError { command: string; response: string }
export interface ApplyResult {
  applied: number;
  errors: CliError[];
  saved: boolean;
  backup: Backup | null;
  reconnected: boolean;
}
export type ConfigStage = "BACKUP" | "SENDING" | "SAVING" | "REBOOTING" | "RECONNECTED";
/** Payload of event `config://progress`. */
export interface ConfigProgress { stage: ConfigStage; fraction: number; message: string | null }
export interface Mismatch { setting: string; expected: string; actual: string }
export interface VerifyReport { ok: boolean; mismatches: Mismatch[] }

// ---- device: motor test + RC stream
/** Clamped by the backend: maxThrottle <= 1250, watchdogMs 100..2000. Defaults 1150 / 500. */
export interface MotorTestLimits { maxThrottle: number; watchdogMs: number }
export interface MotorOutputConfig {
  /** motor_pwm_protocol name, e.g. "DSHOT600", "PWM", "DISABLED". */
  protocol: string;
  digital: boolean;
  feature3d: boolean;
  mincommand: number;
  motorCount: number;
  /** 1000 for DShot/ProShot, else mincommand. */
  stopValue: number;
}
export interface MotorTestSession { outputConfig: MotorOutputConfig; limits: MotorTestLimits }
export type MotorTestStopReason = "REQUESTED" | "WATCHDOG" | "ARMED" | "DISCONNECTED" | "APP_CLOSING" | "ERROR";
/** `device://motor-test-stopped` */
export interface MotorTestStopped { reason: MotorTestStopReason; error: AppError | null }
/** `device://rc-channels`: raw MSP_RC values (usually 1000..2000), firmware channel order. */
export interface RcChannels { seq: number; timestampMs: number; channels: number[] }
export type RcStopReason = "REQUESTED" | "DISCONNECTED" | "ERROR";
/** `device://rc-stopped` */
export interface RcStopped { reason: RcStopReason; error: AppError | null }

/** One sensor in `SensorStatus` (`device_sensors`). */
export interface SensorState {
  /** Detected at boot and running (MSP_STATUS sensor bit). A disabled sensor is never present. */
  present: boolean;
  /** Not disabled in the config (`acc_hardware` / `baro_hardware` / `mag_hardware` != NONE). Always true for the gyro. */
  enabled: boolean;
  /** Configured hardware as the CLI spells it ("AUTO", "NONE", "BMI270", ...). null for the gyro. */
  configuredHardware: string | null;
  /** Chip the FC detected (Betaflight 4.5+), e.g. "BMI270". null on 4.4 / nothing detected. */
  detectedHardware: string | null;
}
/** Board / gyro alignment as stored on the FC (read-only here). */
export interface SensorAlignment {
  /** `align_board_roll` / `_pitch` / `_yaw`, whole degrees. */
  boardRollDeg: number;
  boardPitchDeg: number;
  boardYawDeg: number;
  /** `gyro_1_sensor_align`: "DEFAULT" | "CW0" | "CW90" | "CW180" | "CW270" | "CW0FLIP" | "CW90FLIP" | "CW180FLIP" | "CW270FLIP" | "CUSTOM". */
  gyroAlignment: string;
}
/** `device_sensors` */
export interface SensorStatus {
  gyro: SensorState;
  acc: SensorState;
  baro: SensorState;
  mag: SensorState;
  /** false while Betaflight reports "accelerometer needs calibration"; null if unknown / no acc. */
  accCalibrated: boolean | null;
  /** null if the FC did not answer the alignment requests. */
  alignment: SensorAlignment | null;
}
/** CALIBRATING_FLAG: saw the FC's calibrating flag set then clear. FIXED_WAIT: flag not observable, waited 2 s. */
export type AccCalibrationCompletion = "CALIBRATING_FLAG" | "FIXED_WAIT";
/** `device_acc_calibrate`. The FC has already saved the result to EEPROM. */
export interface AccCalibrationResult {
  durationMs: number;
  completion: AccCalibrationCompletion;
  /** Largest gyro rate seen (deg/s, any axis) during the check and the calibration. */
  maxGyroRateDps: number;
}
/** `device://attitude` (and `device_get_attitude` with seq = timestampMs = 0). Degrees, Betaflight MSP_ATTITUDE signs:
 * roll + = right side down, pitch + = nose DOWN, yaw = heading (+ = nose right). */
export interface Attitude {
  seq: number;
  /** ms since the stream started. */
  timestampMs: number;
  /** -180..180, 0.1° resolution. */
  roll: number;
  /** -90..90, 0.1° resolution. */
  pitch: number;
  /** Heading 0..359, whole degrees. */
  yaw: number;
}
/** `device://attitude-stopped` (same reasons as the RC stream). */
export interface AttitudeStopped { reason: RcStopReason; error: AppError | null }

// ---- system
/** See src-tauri/src/model.rs `DriverStatus` for semantics. */
export interface DriverStatus {
  /** false only when a DFU device is connected without WinUSB -> offer installDfuDriver(). */
  dfuWinusb: boolean;
  /** false only when a VCP / USB-UART bridge is connected without a working driver. */
  vcp: boolean;
  /** English diagnostics for logs/support (not for i18n UI). */
  notes: string[];
  devices: UsbDriverInfo[];
  /** Official vendor driver downloads for USB-UART bridges (always filled). */
  guidance: DriverGuidance[];
}
export type UsbDeviceKind = "DFU" | "FC_VCP" | "USB_UART_BRIDGE";
export type DriverAction = "NONE" | "INSTALL_DFU_DRIVER" | "VENDOR_DOWNLOAD" | "REPLUG";
export interface UsbDriverInfo {
  vid: number;
  pid: number;
  name: string;
  kind: UsbDeviceKind;
  instanceId: string | null;
  service: string | null;
  driverOk: boolean;
  action: DriverAction;
  /** Index into DriverStatus.guidance when action = VENDOR_DOWNLOAD. */
  guidanceIndex: number | null;
  /**
   * DFU only: true when action = INSTALL_DFU_DRIVER and another driver (`service`,
   * e.g. "STTub30" from ST DfuSe, or libusb0) is bound that installing WinUSB will
   * replace. Warn and ask for confirmation before calling installDfuDriver().
   */
  replacesDriver: boolean;
}
export interface DriverGuidance {
  /** Stable id, usable as i18n key suffix: "CP210X" | "CH34X" | "CH343". */
  id: string;
  vendor: string;
  chips: string;
  /** "VID:PID" upper-case hex, e.g. "10C4:EA60". */
  usbIds: string[];
  downloadUrl: string;
}
export interface AppInfo { version: string; os: string; cacheDir: string }

/** Error codes (src-tauri/src/error.rs). The UI maps them to i18n keys `errors.<code>`. */
export type AppErrorCode =
  | "NOT_IMPLEMENTED" | "NOT_CONNECTED" | "SERIAL" | "MSP" | "CLI" | "USB" | "FIRMWARE" | "CONFIG"
  | "UNSUPPORTED_FIRMWARE"
  | "DRIVER" | "IO" | "INVALID_ARGUMENT" | "BOARD_NOT_FOUND" | "PORT_BUSY" | "DRIVER_MISSING" | "DFU_NOT_FOUND"
  | "READ_PROTECTED" | "ERASE_CONFIRMATION_REQUIRED" | "FLASH_FAILED" | "VERIFY_MISMATCH"
  | "MOTOR_TEST_REFUSED" | "BOARD_ARMED" | "MOTOR_TEST_NOT_ACTIVE"
  | "ACC_NOT_AVAILABLE" | "BOARD_MOVING" | "ACC_CALIBRATION_FAILED";
/** `details` of PORT_BUSY. */
export interface PortBusyDetails { port: string; heldBy: "APP" | "EXTERNAL" }
/** `details` of DRIVER_MISSING: pass vid/pid to the driver-install flow. */
export interface DriverMissingDetails { vid: number; pid: number; detail: string }
/** `details` of ACC_NOT_AVAILABLE. */
export interface AccNotAvailableDetails { present: boolean; enabled: boolean }
/** `details` of BOARD_MOVING. `during`: the board moved while calibrating; the stored result is bad, calibrate again. */
export interface BoardMovingDetails { maxRateDps: number; limitDps: number; during: boolean }

/** Error shape returned by every failing command. `details` only for PORT_BUSY / DRIVER_MISSING /
 * ACC_NOT_AVAILABLE / BOARD_MOVING. */
export interface AppError {
  code: string;
  message: string;
  details?: PortBusyDetails | DriverMissingDetails | AccNotAvailableDetails | BoardMovingDetails;
}

export function emptyDroneModel(): DroneModel {
  return {
    schemaVersion: DRONE_MODEL_SCHEMA_VERSION,
    fc: { boardName: "", firmwareVersion: "", apiVersion: "" },
    receiver: { kind: "NONE", protocol: null, port: null, telemetry: true, inverted: false, halfDuplex: false },
    vtx: { kind: "NONE", protocol: null, port: null, digitalSystem: null, region: null },
    gps: { enabled: false, port: null, provider: null },
    esc: { protocol: "DSHOT600", bidirectionalDshot: false, motorPoles: 14, telemetryPort: null, propsOut: false },
    switches: [],
    extras: {},
  };
}
