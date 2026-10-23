// Typed wrapper around Tauri invoke(). The ONLY place the UI talks to Rust.
// Command names and argument names must match src-tauri/src/*/commands.rs.
//
// For tests / browser-only dev, call setBackend(mockBackend) (see ./mock.ts).

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  AppInfo, ApplyResult, Backup, BoardTarget, CliScript, DfuDevice, DriverStatus, DroneModel,
  FcInfo, FcSerialPort, FirmwareImage, FirmwareRelease, FirmwareRequest, FlashOptions, HostPort,
  VerifyReport,
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
  // flasher
  flasher_list_targets: [Record<string, never>, BoardTarget[]];
  flasher_list_releases: [{ target: string }, FirmwareRelease[]];
  flasher_list_dfu_devices: [Record<string, never>, DfuDevice[]];
  flasher_fetch_firmware: [{ request: FirmwareRequest }, FirmwareImage];
  flasher_list_cache: [Record<string, never>, FirmwareImage[]];
  flasher_flash: [{ image: FirmwareImage; dfuSerial: string | null; options: FlashOptions }, void];
  // config
  config_read_ports: [Record<string, never>, FcSerialPort[]];
  config_backup: [Record<string, never>, Backup];
  config_generate: [{ model: DroneModel; ports: FcSerialPort[] }, CliScript];
  config_apply: [{ script: CliScript }, ApplyResult];
  config_verify: [{ model: DroneModel }, VerifyReport];
  config_rollback: [{ backup: Backup }, ApplyResult];
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
  },
  flasher: {
    listTargets: () => call("flasher_list_targets", none),
    listReleases: (target: string) => call("flasher_list_releases", { target }),
    listDfuDevices: () => call("flasher_list_dfu_devices", none),
    fetchFirmware: (request: FirmwareRequest) => call("flasher_fetch_firmware", { request }),
    listCache: () => call("flasher_list_cache", none),
    flash: (image: FirmwareImage, dfuSerial: string | null, options: FlashOptions) =>
      call("flasher_flash", { image, dfuSerial, options }),
  },
  config: {
    readPorts: () => call("config_read_ports", none),
    backup: () => call("config_backup", none),
    generate: (model: DroneModel, ports: FcSerialPort[]) => call("config_generate", { model, ports }),
    apply: (script: CliScript) => call("config_apply", { script }),
    verify: (model: DroneModel) => call("config_verify", { model }),
    rollback: (backup: Backup) => call("config_rollback", { backup }),
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
} as const;
