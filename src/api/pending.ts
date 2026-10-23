// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Typed UI-side wrapper for device commands proposed to Device & Flasher Dev that are
 * not in the shared CommandMap (src/api/commands.ts) yet. Calls go through the same
 * backend seam as `call()`, so the per-call demo fallback, the Demo mode banner and the
 * mixed-backend write guard (api/fallback.ts) apply unchanged.
 *
 * TODO(shared): move these entries to CommandMap / EVENTS when Device's `shared` PR
 * lands, then delete this file. Contract as agreed with Device (2026-10-23):
 *   device_sensors() -> { gyro:{present}, acc:{present,enabled}, baro:{present,enabled},
 *                         mag:{present,enabled}, ...extra keys tolerated }
 *   device_acc_calibrate() -> resolves when done; errors BOARD_ARMED, ACC_NOT_AVAILABLE
 *   device_attitude_stream_start(rateHz?) / device_attitude_stream_stop()
 *     event device://attitude {seq, timestampMs, roll, pitch, yaw} (degrees)
 *     PORT_BUSY {heldBy: APP} while a motor test or the RC stream runs.
 *   No live board-alignment command: the preview rotates the displayed attitude here.
 */
import { call, type CommandName } from "./commands";

export interface SensorInfo {
  present: boolean;
  enabled: boolean;
}
export interface SensorsInfo {
  gyro: { present: boolean };
  acc: SensorInfo;
  baro: SensorInfo;
  mag: SensorInfo;
}
/** `device://attitude` (degrees; roll + = right side down, pitch + = nose up, yaw + = nose right). */
export interface Attitude {
  seq: number;
  timestampMs: number;
  roll: number;
  pitch: number;
  yaw: number;
}

export interface PendingCommandMap {
  device_sensors: [Record<string, never>, unknown];
  device_acc_calibrate: [Record<string, never>, void];
  device_attitude_stream_start: [{ rateHz: number | null }, void];
  device_attitude_stream_stop: [Record<string, never>, void];
}
export type PendingCommandName = keyof PendingCommandMap;

export const SENSORS_COMMAND = "device_sensors";
export const ACC_CALIBRATE_COMMAND = "device_acc_calibrate";
export const ATTITUDE_START_COMMAND = "device_attitude_stream_start";

export const PENDING_EVENTS = { attitude: "device://attitude" } as const;

function callPending<K extends PendingCommandName>(cmd: K, args: PendingCommandMap[K][0]): Promise<PendingCommandMap[K][1]> {
  // The backend seam is untyped at runtime; CommandMap does not know these names yet.
  return call(cmd as unknown as CommandName, args as never) as Promise<PendingCommandMap[K][1]>;
}

const bool = (v: unknown): boolean => v === true;
const sensor = (v: unknown): SensorInfo => {
  const o = v && typeof v === "object" ? (v as Record<string, unknown>) : {};
  const present = bool(o.present);
  return { present, enabled: present && bool(o.enabled) };
};

/** Tolerant parse of device_sensors: unknown keys are ignored, missing sensors = absent. */
export function parseSensors(raw: unknown): SensorsInfo {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const g = o.gyro && typeof o.gyro === "object" ? (o.gyro as Record<string, unknown>) : {};
  return { gyro: { present: bool(g.present) }, acc: sensor(o.acc), baro: sensor(o.baro), mag: sensor(o.mag) };
}

const none = {} as Record<string, never>;

export const pendingApi = {
  sensors: async (): Promise<SensorsInfo> => parseSensors(await callPending("device_sensors", none)),
  /** Writes the calibration to the board: a write command for the mixed-backend guard. */
  accCalibrate: () => callPending("device_acc_calibrate", none),
  attitudeStreamStart: (rateHz: number | null = null) => callPending("device_attitude_stream_start", { rateHz }),
  attitudeStreamStop: () => callPending("device_attitude_stream_stop", none),
};
