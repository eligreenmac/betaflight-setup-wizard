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
  };
  gps: { enabled: boolean; port: string | null; provider: GpsProvider | null };
  esc: {
    protocol: EscProtocol;
    bidirectionalDshot: boolean;
    motorPoles: number | null;
    telemetryPort: string | null;
    propsOut: boolean;
  };
  switches: { mode: string; auxChannel: number; rangeMin: number; rangeMax: number }[];
  extras: { beeper?: boolean | null; ledStrip?: boolean | null; craftName?: string | null };
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
  | "DRIVER" | "IO" | "BOARD_NOT_FOUND" | "PORT_BUSY" | "DRIVER_MISSING" | "DFU_NOT_FOUND"
  | "READ_PROTECTED" | "ERASE_CONFIRMATION_REQUIRED" | "FLASH_FAILED" | "VERIFY_MISMATCH"
  | "MOTOR_TEST_REFUSED" | "BOARD_ARMED" | "MOTOR_TEST_NOT_ACTIVE";
/** `details` of PORT_BUSY. */
export interface PortBusyDetails { port: string; heldBy: "APP" | "EXTERNAL" }
/** `details` of DRIVER_MISSING: pass vid/pid to the driver-install flow. */
export interface DriverMissingDetails { vid: number; pid: number; detail: string }

/** Error shape returned by every failing command. `details` only for PORT_BUSY / DRIVER_MISSING. */
export interface AppError {
  code: string;
  message: string;
  details?: PortBusyDetails | DriverMissingDetails;
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
