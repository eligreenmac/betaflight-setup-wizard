// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Typed UI-side wrapper for device commands proposed to Device & Flasher Dev that are
 * not in the shared CommandMap (src/api/commands.ts) yet. Calls go through the same
 * backend seam as `call()`, so the per-call demo fallback, the Demo mode banner and the
 * mixed-backend write guard (api/fallback.ts) apply unchanged.
 *
 * TODO(shared): move these entries to CommandMap / EVENTS when Device's `shared` PR #63
 * lands, then delete this file. Contract as proposed in #63 (ARCHITECTURE §4.3):
 *   device_sensors() -> SensorStatus {gyro, acc, baro, mag: {present, enabled,
 *     configuredHardware, detectedHardware}, accCalibrated, alignment
 *     {boardRollDeg, boardPitchDeg, boardYawDeg, gyroAlignment} | null}; extra keys tolerated.
 *   device_acc_calibrate() -> {durationMs, completion, maxGyroRateDps}; errors BOARD_ARMED,
 *     ACC_NOT_AVAILABLE {present, enabled}, BOARD_MOVING {maxRateDps, limitDps, during},
 *     ACC_CALIBRATION_FAILED.
 *   device_attitude_stream_start(rateHz?: 10, clamped 1..30) / device_attitude_stream_stop()
 *     event device://attitude {seq, timestampMs, roll, pitch, yaw} (degrees, MSP_ATTITUDE
 *     signs: roll + = right side down, pitch + = nose DOWN, yaw = heading),
 *     event device://attitude-stopped {reason, error};
 *     PORT_BUSY {heldBy: APP} while a motor test or the RC stream runs (and vice versa).
 *   No live board-alignment command: the preview rotates the displayed attitude here.
 */
import { call, type CommandName } from "./commands";

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
/**
 * `device://attitude` as sent (MSP_ATTITUDE signs: roll + = right side down,
 * pitch + = nose DOWN, yaw = heading, + = nose right). Use `attitudeToEuler` before
 * the orientation math (which uses pitch + = nose up).
 */
export interface Attitude {
  seq: number;
  timestampMs: number;
  roll: number;
  pitch: number;
  yaw: number;
}

export interface PendingCommandMap {
  device_sensors: [Record<string, never>, unknown];
  device_acc_calibrate: [Record<string, never>, unknown];
  device_attitude_stream_start: [{ rateHz: number | null }, void];
  device_attitude_stream_stop: [Record<string, never>, void];
}
export type PendingCommandName = keyof PendingCommandMap;

export const SENSORS_COMMAND = "device_sensors";
export const ACC_CALIBRATE_COMMAND = "device_acc_calibrate";
export const ATTITUDE_START_COMMAND = "device_attitude_stream_start";

export const PENDING_EVENTS = { attitude: "device://attitude", attitudeStopped: "device://attitude-stopped" } as const;

/** `device://attitude-stopped` */
export interface AttitudeStopped {
  reason: "REQUESTED" | "DISCONNECTED" | "ERROR";
  error: { code: string; message: string; details?: unknown } | null;
}

/** MSP_ATTITUDE (pitch + = nose down) -> the orientation math's Euler (pitch + = nose up). */
export function attitudeToEuler(a: Pick<Attitude, "roll" | "pitch" | "yaw">): { roll: number; pitch: number; yaw: number } {
  return { roll: a.roll, pitch: a.pitch === 0 ? 0 : -a.pitch, yaw: a.yaw };
}

function callPending<K extends PendingCommandName>(cmd: K, args: PendingCommandMap[K][0]): Promise<PendingCommandMap[K][1]> {
  // The backend seam is untyped at runtime; CommandMap does not know these names yet.
  return call(cmd as unknown as CommandName, args as never) as Promise<PendingCommandMap[K][1]>;
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

const none = {} as Record<string, never>;

export const pendingApi = {
  sensors: async (): Promise<SensorsInfo> => parseSensors(await callPending("device_sensors", none)),
  /** Writes the calibration to the board: a write command for the mixed-backend guard. */
  accCalibrate: () => callPending("device_acc_calibrate", none),
  attitudeStreamStart: (rateHz: number | null = null) => callPending("device_attitude_stream_start", { rateHz }),
  attitudeStreamStop: () => callPending("device_attitude_stream_stop", none),
};
