// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UI adapter for the Device sensors / calibration / attitude commands of shared PR #63
 * (now in CommandMap: api.device.sensors / accCalibrate / attitudeStream*, EVENTS.attitude*).
 * Calls go through `call()`, so the per-call demo fallback, the Demo mode banner and the
 * mixed-backend write guard (api/fallback.ts) apply unchanged. What this file adds:
 *   - a tolerant parse of SensorStatus (extra keys ignored, missing sensors = absent) into
 *     the small shape the UI uses, with the alignment as {roll, pitch, yaw};
 *   - attitudeToEuler: MSP_ATTITUDE signs (pitch + = nose DOWN) -> the orientation math.
 * No live board-alignment command: the preview rotates the displayed attitude here.
 */
import { api, call, EVENTS, type CommandName } from "./commands";
import type { Attitude, AttitudeStopped } from "../types/model";

export type { Attitude, AttitudeStopped };

export interface SensorInfo {
  present: boolean;
  enabled: boolean;
}
/** Board alignment stored on the FC (`align_board_*`, degrees). */
export interface CurrentAlignment {
  roll: number;
  pitch: number;
  yaw: number;
}
export interface SensorsInfo {
  gyro: { present: boolean };
  acc: SensorInfo;
  baro: SensorInfo;
  mag: SensorInfo;
  /** false = Betaflight says "accelerometer needs calibration"; null = unknown. */
  accCalibrated: boolean | null;
  /** The FC's current board alignment; null = not reported. */
  alignment: CurrentAlignment | null;
}
/** Command names the mock and the guards refer to. */
export const PENDING_COMMANDS = ["device_reboot", "device_sensors", "device_acc_calibrate", "device_get_attitude", "device_attitude_stream_start", "device_attitude_stream_stop"] as const;
export type PendingCommandName = (typeof PENDING_COMMANDS)[number];

export const SENSORS_COMMAND = "device_sensors";
export const ACC_CALIBRATE_COMMAND = "device_acc_calibrate";
export const ATTITUDE_START_COMMAND = "device_attitude_stream_start";

/**
 * TODO(shared): Device's `device_reboot(): Promise<void>` (follow-up to #68/#69) is not in
 * CommandMap yet. It reboots the board, waits for it to come back, re-verifies the API
 * version + UID, clears the unsaved-changes flag and resolves; it rejects when the reboot
 * cannot be confirmed. Called through `call()` so the demo fallback and the mixed-backend
 * guard (it is a write: api/fallback.ts) apply. Swap for api.device.reboot when it lands.
 */
export const DEVICE_REBOOT_COMMAND = "device_reboot";
export const deviceRebootApi = {
  reboot: (): Promise<void> => call(DEVICE_REBOOT_COMMAND as unknown as CommandName, {} as never) as Promise<void>,
};

export const PENDING_EVENTS = { attitude: EVENTS.attitude, attitudeStopped: EVENTS.attitudeStopped } as const;

/** MSP_ATTITUDE (pitch + = nose down) -> the orientation math's Euler (pitch + = nose up). */
export function attitudeToEuler(a: Pick<Attitude, "roll" | "pitch" | "yaw">): { roll: number; pitch: number; yaw: number } {
  return { roll: a.roll, pitch: a.pitch === 0 ? 0 : -a.pitch, yaw: a.yaw };
}

const bool = (v: unknown): boolean => v === true;
// present = detected and running; enabled = not NONE in the config. A disabled sensor is
// never present, so !present && !enabled = "turned off" (can be turned on), and
// !present && enabled = "not found".
const sensor = (v: unknown): SensorInfo => {
  const o = v && typeof v === "object" ? (v as Record<string, unknown>) : {};
  const present = bool(o.present);
  return { present, enabled: present || o.enabled !== false };
};

const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);
function alignment(v: unknown): CurrentAlignment | null {
  const o = v && typeof v === "object" ? (v as Record<string, unknown>) : null;
  if (!o) return null;
  const [roll, pitch, yaw] = [num(o.boardRollDeg), num(o.boardPitchDeg), num(o.boardYawDeg)];
  return roll === null || pitch === null || yaw === null ? null : { roll, pitch, yaw };
}

/** Tolerant parse of device_sensors: unknown keys are ignored, missing sensors = absent. */
export function parseSensors(raw: unknown): SensorsInfo {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const g = o.gyro && typeof o.gyro === "object" ? (o.gyro as Record<string, unknown>) : {};
  return {
    gyro: { present: bool(g.present) },
    acc: sensor(o.acc),
    baro: sensor(o.baro),
    mag: sensor(o.mag),
    accCalibrated: typeof o.accCalibrated === "boolean" ? o.accCalibrated : null,
    alignment: alignment(o.alignment),
  };
}

export const pendingApi = {
  sensors: async (): Promise<SensorsInfo> => parseSensors(await api.device.sensors()),
  /** Writes the calibration to the board (the FC saves it to EEPROM itself): a write command for the mixed-backend guard. */
  accCalibrate: () => api.device.accCalibrate(),
  attitudeStreamStart: (rateHz: number | null = null) => api.device.attitudeStreamStart(rateHz),
  attitudeStreamStop: () => api.device.attitudeStreamStop(),
};
