// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Device contract of shared PR #38 (feat/shared-device-contracts), mirrored UI-side
 * so the wizard can code against it before it lands on main.
 *
 * TODO(shared #38): once #38 is on main, replace `device.*` with `api.device.*`,
 * DEVICE_EVENTS with EVENTS, import these types from types/model.ts and delete this
 * file. Names, argument shapes and payloads below are copied from #38 verbatim:
 *   - device_* commands without a port (the connection manager holds it, §4.1);
 *   - MotorTestLimits {maxThrottle, watchdogMs}; SCREAMING_SNAKE stop reasons;
 *   - AppError.details: PORT_BUSY {port, heldBy}, DRIVER_MISSING {vid, pid, detail};
 *   - flasher_remove_read_protection {dfuSerial, confirmErase} (§2.3 / §6 step 5): erases the
 *     whole chip; the backend refuses with ERASE_CONFIRMATION_REQUIRED unless confirmErase.
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { FcInfo, FcSerialPort } from "../types/model";

export interface MotorTestLimits {
  /** Default 1150, hard cap 1250. Over-cap requests are refused, not clamped. */
  maxThrottle: number;
  /** Backend watchdog: no keepalive within this many ms stops the motors. Default 500, clamped 100..2000. */
  watchdogMs: number;
}
export interface MotorOutputConfig {
  protocol: string;
  digital: boolean;
  feature3d: boolean;
  mincommand: number;
  motorCount: number;
  stopValue: number;
}
export interface MotorTestSession {
  outputConfig: MotorOutputConfig;
  limits: MotorTestLimits;
}
export type MotorTestStopReason = "REQUESTED" | "WATCHDOG" | "ARMED" | "DISCONNECTED" | "APP_CLOSING" | "ERROR";
export interface DeviceAppError {
  code: string;
  message: string;
  details?: PortBusyDetails | DriverMissingDetails;
}
/** `device://motor-test-stopped` */
export interface MotorTestStopped {
  reason: MotorTestStopReason;
  error: DeviceAppError | null;
}
/** `device://rc-channels`: raw MSP_RC values, firmware channel order. */
export interface RcChannels {
  seq: number;
  timestampMs: number;
  channels: number[];
}
export type RcStopReason = "REQUESTED" | "DISCONNECTED" | "ERROR";
/** `device://rc-stopped` */
export interface RcStopped {
  reason: RcStopReason;
  error: DeviceAppError | null;
}
/** Onboard SPI receiver. `present` is null until device_probe_board ran. */
export interface RxSpiInfo {
  present: boolean | null;
  expresslrsPins: boolean;
  protocol: string | null;
  featureEnabled: boolean | null;
  resources: string[];
}
export interface PortBusyDetails {
  port: string;
  heldBy: "APP" | "EXTERNAL";
}
export interface DriverMissingDetails {
  vid: number;
  pid: number;
  detail: string;
}
/** FcInfo as #38 extends it (optional here: today's backend does not send them). */
export type DeviceFcInfo = FcInfo & { serialPorts?: FcSerialPort[]; rxSpi?: RxSpiInfo };

type None = Record<string, never>;
export interface DeviceCommandMap {
  device_probe_board: [None, DeviceFcInfo];
  device_motor_test_start: [{ motorIndex: number | null; throttle: number; propsOffConfirmed: boolean; limits: MotorTestLimits | null }, MotorTestSession];
  device_motor_test_keepalive: [None, void];
  device_motor_test_stop: [None, void];
  device_rc_stream_start: [{ rateHz: number | null }, void];
  device_rc_stream_stop: [None, void];
  flasher_remove_read_protection: [{ dfuSerial: string | null; confirmErase: boolean }, void];
}
export type DeviceCommand = keyof DeviceCommandMap;
export type DeviceBackend = <K extends DeviceCommand>(cmd: K, args: DeviceCommandMap[K][0]) => Promise<DeviceCommandMap[K][1]>;

const tauriDeviceBackend: DeviceBackend = (cmd, args) => tauriInvoke(cmd, args as Record<string, unknown>);
let deviceBackend: DeviceBackend = tauriDeviceBackend;
export function setDeviceBackend(b: DeviceBackend): void {
  deviceBackend = b;
}
export function resetDeviceBackend(): void {
  deviceBackend = tauriDeviceBackend;
}
export const tauriDevice = tauriDeviceBackend;

const none: None = {};
const call = <K extends DeviceCommand>(cmd: K, args: DeviceCommandMap[K][0]) => deviceBackend(cmd, args);

export const device = {
  probeBoard: () => call("device_probe_board", none),
  motorTestStart: (motorIndex: number | null, throttle: number, propsOffConfirmed: boolean, limits: MotorTestLimits | null = null) =>
    call("device_motor_test_start", { motorIndex, throttle, propsOffConfirmed, limits }),
  motorTestKeepalive: () => call("device_motor_test_keepalive", none),
  motorTestStop: () => call("device_motor_test_stop", none),
  rcStreamStart: (rateHz: number | null = null) => call("device_rc_stream_start", { rateHz }),
  rcStreamStop: () => call("device_rc_stream_stop", none),
};

/** #38 flasher additions (TODO(shared #38): move to api.flasher). */
export const flasherNext = {
  /** ERASES THE WHOLE CHIP. Only call after the user explicitly confirmed (ReadProtectedPanel). */
  removeReadProtection: (dfuSerial: string | null, confirmErase: boolean) =>
    call("flasher_remove_read_protection", { dfuSerial, confirmErase }),
};

export const DEVICE_EVENTS = {
  motorTestStopped: "device://motor-test-stopped",
  rcChannels: "device://rc-channels",
  rcStopped: "device://rc-stopped",
} as const;
