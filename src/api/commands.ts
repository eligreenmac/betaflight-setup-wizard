// Typed wrapper around Tauri invoke(). The ONLY place the UI talks to Rust.
// Command names and argument names must match src-tauri/src/*/commands.rs.
//
// For tests / browser-only dev, call setBackend(mockBackend) (see ./mock.ts).

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  AccCalibrationResult, AppInfo, ApplyResult, Attitude, Backup, BoardTarget, CliScript, DfuDevice, DriverStatus, DroneModel,
  FcInfo, FcSerialPort, FirmwareCatalog, FirmwareImage, MotorTestLimits, MotorTestSession, FirmwareRelease, FirmwareRequest, FlashOptions, HostPort,
  OsdLayout, OsdVideoSystem, SensorStatus, VerifyReport,
} from "../types/model";

/** Map of command name -> [args, result]. Single source of truth on the TS side. */
export interface CommandMap {
  // device
  device_list_ports: [Record<string, never>, HostPort[]];
  device_connect: [{ path: string }, void];
  device_disconnect: [Record<string, never>, void];
  device_identify: [Record<string, never>, FcInfo];
  device_cli_exec: [{ line: string }, string];
  device_reboot_to_bootloader: [Record<string, never>, void];
  device_reboot: [Record<string, never>, void];
  device_probe_board: [Record<string, never>, FcInfo];
  device_motor_test_start: [
    { motorIndex: number | null; throttle: number; propsOffConfirmed: boolean; limits: MotorTestLimits | null },
    MotorTestSession,
  ];
  device_motor_test_keepalive: [Record<string, never>, void];
  device_motor_test_stop: [Record<string, never>, void];
  device_rc_stream_start: [{ rateHz: number | null }, void];
  device_rc_stream_stop: [Record<string, never>, void];
  device_sensors: [Record<string, never>, SensorStatus];
  device_acc_calibrate: [Record<string, never>, AccCalibrationResult];
  device_get_attitude: [Record<string, never>, Attitude];
  device_attitude_stream_start: [{ rateHz: number | null }, void];
  device_attitude_stream_stop: [Record<string, never>, void];
  // flasher
  flasher_list_targets: [Record<string, never>, BoardTarget[]];
  flasher_list_releases: [{ target: string }, FirmwareRelease[]];
  flasher_list_dfu_devices: [Record<string, never>, DfuDevice[]];
  flasher_fetch_firmware: [{ request: FirmwareRequest }, FirmwareImage];
  flasher_list_cache: [Record<string, never>, FirmwareImage[]];
  flasher_flash: [{ image: FirmwareImage; dfuSerial: string | null; options: FlashOptions }, void];
  flasher_remove_read_protection: [{ dfuSerial: string | null; confirmErase: boolean }, void];
  // config
  config_read_ports: [Record<string, never>, FcSerialPort[]];
  config_backup: [Record<string, never>, Backup];
  /** Preview only: never apply this script; apply the one from config_plan. */
  config_generate: [{ model: DroneModel; ports: FcSerialPort[] }, CliScript];
  /** Binding phrase -> ExpressLRS UID. Pure; the phrase is not stored or logged. */
  config_elrs_uid: [{ phrase: string }, number[]];
  config_apply: [{ script: CliScript }, ApplyResult];
  config_verify: [{ model: DroneModel }, VerifyReport];
  config_plan: [{ model: DroneModel }, CliScript];
  config_rollback: [{ backup: Backup }, ApplyResult];
  config_list_backups: [Record<string, never>, Backup[]];
  config_backups_dir: [Record<string, never>, string];
  config_restore: [{ backupId: string }, ApplyResult];
  /** Pure: config's default OSD layout for that video system and firmware line. */
  config_osd_default_layout: [{ videoSystem: OsdVideoSystem; firmwareVersion: string }, OsdLayout];
  /** Pure: per-firmware beepers / beacon events / OSD elements / mode names. */
  config_firmware_catalog: [{ firmwareVersion: string }, FirmwareCatalog];
  // system
  system_driver_status: [Record<string, never>, DriverStatus];
  system_install_dfu_driver: [Record<string, never>, void];
  system_app_info: [Record<string, never>, AppInfo];
}

export type CommandName = keyof CommandMap;
export type Backend = <K extends CommandName>(cmd: K, args: CommandMap[K][0]) => Promise<CommandMap[K][1]>;

const tauriBackend: Backend = (cmd, args) => tauriInvoke(cmd, args as Record<string, unknown>);

let backend: Backend = tauriBackend;

/** Replace the backend (tests, browser dev mode). */
export function setBackend(b: Backend): void {
  backend = b;
}
export function resetBackend(): void {
  backend = tauriBackend;
}

export function call<K extends CommandName>(cmd: K, args: CommandMap[K][0]): Promise<CommandMap[K][1]> {
  return backend(cmd, args);
}

const none = {} as Record<string, never>;

/** Convenience API grouped by workstream. */
export const api = {
  device: {
    listPorts: () => call("device_list_ports", none),
    connect: (path: string) => call("device_connect", { path }),
    disconnect: () => call("device_disconnect", none),
    identify: () => call("device_identify", none),
    cliExec: (line: string) => call("device_cli_exec", { line }),
    rebootToBootloader: () => call("device_reboot_to_bootloader", none),
    /** "Reboot board": app-issued firmware reboot, resolves once it is confirmed (ack + same board back); clears PENDING_UNSAVED_CHANGES. PENDING_UNSAVED_CHANGES / MSP if not confirmed, SERIAL if the board did not come back. */
    reboot: () => call("device_reboot", none),
    probeBoard: () => call("device_probe_board", none),
    /** First call starts the test, later calls change the throttle. Feeds the watchdog. */
    motorTestStart: (motorIndex: number | null, throttle: number, propsOffConfirmed: boolean, limits: MotorTestLimits | null = null) =>
      call("device_motor_test_start", { motorIndex, throttle, propsOffConfirmed, limits }),
    /** Every 200 ms while a test runs. */
    motorTestKeepalive: () => call("device_motor_test_keepalive", none),
    motorTestStop: () => call("device_motor_test_stop", none),
    rcStreamStart: (rateHz: number | null = null) => call("device_rc_stream_start", { rateHz }),
    rcStreamStop: () => call("device_rc_stream_stop", none),
    /** Detected / enabled state of gyro, acc, baro, mag (+ hardware names, alignment readout). Read-only. */
    sensors: () => call("device_sensors", none),
    /** Board level and still. Resolves when done (FC saves by itself). PENDING_UNSAVED_CHANGES (checked first) / BOARD_ARMED / ACC_NOT_AVAILABLE / BOARD_MOVING / ACC_CALIBRATION_FAILED. */
    accCalibrate: () => call("device_acc_calibrate", none),
    getAttitude: () => call("device_get_attitude", none),
    /** Default 10 Hz, clamped 1..30. Exclusive with the motor test and the RC stream (PORT_BUSY). */
    attitudeStreamStart: (rateHz: number | null = null) => call("device_attitude_stream_start", { rateHz }),
    attitudeStreamStop: () => call("device_attitude_stream_stop", none),
  },
  flasher: {
    listTargets: () => call("flasher_list_targets", none),
    listReleases: (target: string) => call("flasher_list_releases", { target }),
    listDfuDevices: () => call("flasher_list_dfu_devices", none),
    fetchFirmware: (request: FirmwareRequest) => call("flasher_fetch_firmware", { request }),
    listCache: () => call("flasher_list_cache", none),
    flash: (image: FirmwareImage, dfuSerial: string | null, options: FlashOptions) =>
      call("flasher_flash", { image, dfuSerial, options }),
    /** Erases the whole chip. Only after the user explicitly confirmed. */
    removeReadProtection: (dfuSerial: string | null, confirmErase: boolean) =>
      call("flasher_remove_read_protection", { dfuSerial, confirmErase }),
  },
  config: {
    readPorts: () => call("config_read_ports", none),
    backup: () => call("config_backup", none),
    /** Preview only (assumes default bauds, free aux slots); apply the script from `plan`. */
    generate: (model: DroneModel, ports: FcSerialPort[]) => call("config_generate", { model, ports }),
    /** Hash a binding phrase to the 6-byte UID. Drop the phrase afterwards. */
    elrsUid: (phrase: string) => call("config_elrs_uid", { phrase }),
    apply: (script: CliScript) => call("config_apply", { script }),
    verify: (model: DroneModel) => call("config_verify", { model }),
    plan: (model: DroneModel) => call("config_plan", { model }),
    rollback: (backup: Backup) => call("config_rollback", { backup }),
    listBackups: () => call("config_list_backups", none),
    backupsDir: () => call("config_backups_dir", none),
    restore: (backupId: string) => call("config_restore", { backupId }),
    osdDefaultLayout: (videoSystem: OsdVideoSystem, firmwareVersion: string) =>
      call("config_osd_default_layout", { videoSystem, firmwareVersion }),
    firmwareCatalog: (firmwareVersion: string) => call("config_firmware_catalog", { firmwareVersion }),
  },
  system: {
    driverStatus: () => call("system_driver_status", none),
    installDfuDriver: () => call("system_install_dfu_driver", none),
    appInfo: () => call("system_app_info", none),
  },
};

/** Event names emitted by Rust. */
export const EVENTS = {
  flashProgress: "flasher://progress",
  configProgress: "config://progress",
  motorTestStopped: "device://motor-test-stopped",
  rcChannels: "device://rc-channels",
  rcStopped: "device://rc-stopped",
  attitude: "device://attitude",
  attitudeStopped: "device://attitude-stopped",
} as const;
