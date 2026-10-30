// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Pure math for the live 3D quad in the Calibration step (no three.js here, so it is
 * testable in jsdom and the three.js chunk stays lazy).
 *
 * Input: the orientation math's Euler (wizard/orientation.ts, aerospace NED, ZYX):
 * roll + = right side down, pitch + = nose UP, yaw + = nose right. The live stream is
 * MSP_ATTITUDE (pitch + = nose DOWN): convert with api/pending `attitudeToEuler` first,
 * then apply the board-alignment preview (orientation.ts `alignedAttitude`).
 *
 * Scene frame (three.js, viewed like Betaflight Configurator: from behind and above):
 * +X = the quad's right, +Y = up, -Z = forward (the nose points away from the viewer).
 * NED (x forward, y right, z down) -> scene: P = [[0,1,0],[0,0,-1],[-1,0,0]], and the
 * body rotation in scene coordinates is R_scene = P · R_ned · Pᵀ.
 */
import { toMatrix, type Euler } from "./orientation";

export type Mat3 = [number, number, number, number, number, number, number, number, number];
export type Vec3 = [number, number, number];
export interface Quat {
  x: number;
  y: number;
  z: number;
  w: number;
}

/**
 * Attitude stream rate while the orientation check is on screen. 30 Hz is Device's default
 * for the 3D view (Device #94: default 30, clamped 1..50; latest sample only, so `seq` may skip).
 * The stream runs only while the step is visible (stopped on leave and while the window is hidden).
 */
export const ATTITUDE_RATE_HZ = 30;

const P: Mat3 = [0, 1, 0, 0, 0, -1, -1, 0, 0];
const T = (m: Mat3): Mat3 => [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];
function mul(a: Mat3, b: Mat3): Mat3 {
  const o = [0, 0, 0, 0, 0, 0, 0, 0, 0] as Mat3;
  for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) for (let k = 0; k < 3; k++) o[i * 3 + j] += a[i * 3 + k]! * b[k * 3 + j]!;
  return o;
}

/** Body -> scene rotation (row-major) for an orientation-math Euler. */
export function sceneMatrix(e: Euler): Mat3 {
  return mul(mul(P, toMatrix(e) as Mat3), T(P));
}

export function apply(m: Mat3, v: Vec3): Vec3 {
  return [m[0] * v[0] + m[1] * v[1] + m[2] * v[2], m[3] * v[0] + m[4] * v[1] + m[5] * v[2], m[6] * v[0] + m[7] * v[1] + m[8] * v[2]];
}

/** Model axes in the scene at rest. */
export const NOSE: Vec3 = [0, 0, -1];
export const RIGHT: Vec3 = [1, 0, 0];
export const UP: Vec3 = [0, 1, 0];

/** Unit quaternion of a rotation matrix (three.js Quaternion.set(x, y, z, w) order). */
export function matrixToQuat(m: Mat3): Quat {
  const [m00, m01, m02, m10, m11, m12, m20, m21, m22] = m;
  const tr = m00 + m11 + m22;
  let q: Quat;
  if (tr > 0) {
    const s = 0.5 / Math.sqrt(tr + 1);
    q = { w: 0.25 / s, x: (m21 - m12) * s, y: (m02 - m20) * s, z: (m10 - m01) * s };
  } else if (m00 > m11 && m00 > m22) {
    const s = 2 * Math.sqrt(1 + m00 - m11 - m22);
    q = { w: (m21 - m12) / s, x: 0.25 * s, y: (m01 + m10) / s, z: (m02 + m20) / s };
  } else if (m11 > m22) {
    const s = 2 * Math.sqrt(1 + m11 - m00 - m22);
    q = { w: (m02 - m20) / s, x: (m01 + m10) / s, y: 0.25 * s, z: (m12 + m21) / s };
  } else {
    const s = 2 * Math.sqrt(1 + m22 - m00 - m11);
    q = { w: (m10 - m01) / s, x: (m02 + m20) / s, y: (m12 + m21) / s, z: 0.25 * s };
  }
  return q;
}

/** Scene quaternion for an orientation-math Euler. */
export const sceneQuat = (e: Euler): Quat => matrixToQuat(sceneMatrix(e));

/**
 * Fraction of the way to move toward the newest sample this frame (exponential smoothing,
 * frame-rate independent). Reduced motion = jump straight to the sample, no easing.
 */
export function smoothingStep(dtMs: number, reducedMotion: boolean, tauMs = 70): number {
  if (reducedMotion || dtMs <= 0) return reducedMotion ? 1 : 0;
  return 1 - Math.exp(-dtMs / tauMs);
}

/** Heading relative to `refYaw` (the "face forward" reference), in [0, 360). */
export function relativeYaw(e: Euler, refYaw: number): Euler {
  return { ...e, yaw: (((e.yaw - refYaw) % 360) + 360) % 360 };
}

/** Can this browser draw WebGL? false in jsdom (no WebGLRenderingContext) and when the GPU is blocked. */
export function webglAvailable(): boolean {
  try {
    if (typeof window === "undefined" || typeof (globalThis as { WebGLRenderingContext?: unknown }).WebGLRenderingContext === "undefined") return false;
    const c = document.createElement("canvas");
    return !!(c.getContext("webgl2") ?? c.getContext("webgl"));
  } catch {
    return false;
  }
}

/** prefers-reduced-motion: reduce (false when matchMedia is missing, e.g. jsdom). */
export function prefersReducedMotion(): boolean {
  try {
    return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}
