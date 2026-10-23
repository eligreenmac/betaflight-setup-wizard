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

// ---- system
export interface DriverStatus { dfuWinusb: boolean; vcp: boolean; notes: string[] }
export interface AppInfo { version: string; os: string; cacheDir: string }

/** Error codes (src-tauri/src/error.rs). The UI maps them to i18n keys `errors.<code>`. */
export type AppErrorCode =
  | "NOT_IMPLEMENTED" | "NOT_CONNECTED" | "SERIAL" | "MSP" | "CLI" | "USB" | "FIRMWARE" | "CONFIG"
  | "DRIVER" | "IO" | "BOARD_NOT_FOUND" | "PORT_BUSY" | "DRIVER_MISSING" | "DFU_NOT_FOUND"
  | "READ_PROTECTED" | "ERASE_CONFIRMATION_REQUIRED" | "FLASH_FAILED" | "VERIFY_MISMATCH";
/** `details` of PORT_BUSY. */
export interface PortBusyDetails { port: string; heldBy: "app" | "external" }
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
