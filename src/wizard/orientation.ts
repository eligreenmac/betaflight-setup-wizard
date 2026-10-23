// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Board alignment preview math. There is no live board-alignment command, so the
 * Calibration step previews a candidate alignment by rotating the attitude the board
 * reports, client-side. Saved alignment goes through the config script (boardAlignment).
 *
 * Conventions (aerospace, NED; also Betaflight's): roll + = right side down,
 * pitch + = nose up, yaw + = nose right (clockwise seen from above). Euler order ZYX.
 * align_board_* = how the board is turned relative to the frame, e.g. yaw 90 = board
 * arrow points to the right; roll 180 = mounted upside down.
 *
 * The board reports the attitude of its own (sensor) frame: R_reported = R_body · A,
 * where A is the mounting rotation. Previewing alignment A gives R_body = R_reported · Aᵀ.
 * ASSUMPTION (TODO(shared)): the stream reports attitude with the FC's current alignment
 * at 0/0/0 (a fresh board); a preset is then the absolute value to save.
 */
import type { BoardAlignment } from "./modelExt";

export interface Euler {
  roll: number;
  pitch: number;
  yaw: number;
}
type Mat = [number, number, number, number, number, number, number, number, number];

const rad = (d: number) => (d * Math.PI) / 180;
const deg = (r: number) => (r * 180) / Math.PI;

/** Body -> earth rotation matrix (row-major) from ZYX Euler angles in degrees. */
export function toMatrix(e: Euler): Mat {
  const [cr, sr, cp, sp, cy, sy] = [Math.cos(rad(e.roll)), Math.sin(rad(e.roll)), Math.cos(rad(e.pitch)), Math.sin(rad(e.pitch)), Math.cos(rad(e.yaw)), Math.sin(rad(e.yaw))];
  return [
    cy * cp, cy * sp * sr - sy * cr, cy * sp * cr + sy * sr,
    sy * cp, sy * sp * sr + cy * cr, sy * sp * cr - cy * sr,
    -sp, cp * sr, cp * cr,
  ];
}

/** Inverse of toMatrix; angles in (-180, 180], yaw normalised to [0, 360). */
export function toEuler(m: Mat): Euler {
  const pitch = deg(Math.asin(Math.max(-1, Math.min(1, -m[6]))));
  const roll = deg(Math.atan2(m[7], m[8]));
  const yaw = deg(Math.atan2(m[3], m[0]));
  return { roll: clean(roll), pitch: clean(pitch), yaw: clean(((yaw % 360) + 360) % 360) };
}
const clean = (v: number) => {
  const r = Math.round(v * 1000) / 1000;
  return Object.is(r, -0) || Math.abs(r) < 1e-9 ? 0 : r === -180 ? 180 : r;
};

function mul(a: Mat, b: Mat): Mat {
  const o = new Array(9).fill(0) as Mat;
  for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) for (let k = 0; k < 3; k++) o[i * 3 + j] += a[i * 3 + k]! * b[k * 3 + j]!;
  return o;
}
const transpose = (m: Mat): Mat => [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];

/** What the board reports when the frame has attitude `body` and the board is mounted `mount`. */
export function reportedAttitude(body: Euler, mount: BoardAlignment): Euler {
  return toEuler(mul(toMatrix(body), toMatrix(mount)));
}

/** Preview: the frame's attitude if the board's alignment were `align`. */
export function alignedAttitude(reported: Euler, align: BoardAlignment): Euler {
  return toEuler(mul(toMatrix(reported), transpose(toMatrix(align))));
}

export interface AlignmentPreset {
  id: string;
  align: BoardAlignment;
}
/** Common mountings: arrow forward / right / back / left, each also upside down. */
export const ALIGNMENT_PRESETS: AlignmentPreset[] = [
  { id: "default", align: { roll: 0, pitch: 0, yaw: 0 } },
  { id: "yaw90", align: { roll: 0, pitch: 0, yaw: 90 } },
  { id: "yaw180", align: { roll: 0, pitch: 0, yaw: 180 } },
  { id: "yaw270", align: { roll: 0, pitch: 0, yaw: 270 } },
  { id: "roll180", align: { roll: 180, pitch: 0, yaw: 0 } },
  { id: "roll180yaw90", align: { roll: 180, pitch: 0, yaw: 90 } },
  { id: "roll180yaw180", align: { roll: 180, pitch: 0, yaw: 180 } },
  { id: "roll180yaw270", align: { roll: 180, pitch: 0, yaw: 270 } },
];

export const sameAlignment = (a: BoardAlignment | null, b: BoardAlignment): boolean => !!a && a.roll === b.roll && a.pitch === b.pitch && a.yaw === b.yaw;

/** Which way the frame is tilted, for "did the picture move the same way?" checks. */
export function tiltDirection(e: Euler, threshold = 15): "noseDown" | "noseUp" | "rightDown" | "leftDown" | "level" {
  const r = ((e.roll + 540) % 360) - 180;
  const p = e.pitch;
  if (Math.abs(p) >= Math.abs(r) && Math.abs(p) >= threshold) return p < 0 ? "noseDown" : "noseUp";
  if (Math.abs(r) >= threshold) return r > 0 ? "rightDown" : "leftDown";
  return "level";
}
