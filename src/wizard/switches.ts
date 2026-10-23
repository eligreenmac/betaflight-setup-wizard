// SPDX-License-Identifier: GPL-3.0-or-later
/** Switch helpers: beginners pick "low / middle / high", DroneModel stores PWM ranges. */
export const SWITCH_POSITIONS = {
  low: [900, 1300],
  mid: [1300, 1700],
  high: [1700, 2100],
} as const;
export type SwitchPosition = keyof typeof SWITCH_POSITIONS;

export function positionOf(rangeMin: number, rangeMax: number): SwitchPosition | null {
  for (const [k, [s, e]] of Object.entries(SWITCH_POSITIONS)) if (s === rangeMin && e === rangeMax) return k as SwitchPosition;
  return null;
}

export function inRange(value: number, rangeMin: number, rangeMax: number): boolean {
  return value >= rangeMin && value < rangeMax;
}

/** DroneModel.switches[].auxChannel is 0-based (AUX1 = 0). AUX1 is RC channel 5, i.e. index 4. */
export const auxToChannelIndex = (aux: number): number => aux + 4;

import { compareVersions } from "./version";

const squash = (s: string) => s.toUpperCase().replace(/[^A-Z0-9]/g, "");

/**
 * Modes the wizard offers. DroneModel.switches[].mode stores only the STABLE mode name
 * (schemas/drone-model.schema.json, config::modes canonical names: "ARM", "ANGLE",
 * "BEEPER", "FLIP OVER AFTER CRASH", "ALTHOLD", "POSHOLD"). Never box ids, never a
 * firmware's display name: config::modes maps stable names to box ids per firmware.
 * Only the labels shown to the user depend on the firmware version.
 * PREARM and AIR MODE use config::modes' canonical spelling on main ("PREARM",
 * "AIR MODE"); its resolver also accepts "AIRMODE" / "FLIP_OVER_AFTER_CRASH".
 */
export const MODE_KEYS = ["arm", "angle", "horizon", "beeper", "turtle", "altHold", "posHold", "gpsRescue", "prearm", "airMode"] as const;
export type ModeKey = (typeof MODE_KEYS)[number];
export const OPTIONAL_MODES: ModeKey[] = ["angle", "horizon", "beeper", "turtle", "altHold", "posHold", "gpsRescue", "prearm", "airMode"];
/** Modes that need the accelerometer (self-levelling / GPS): acc:false with one of these is an error. */
export const ACC_MODES: ModeKey[] = ["angle", "horizon", "altHold", "posHold", "gpsRescue"];
/** Modes that need the barometer to work well (baro:false with one of these is a warning). */
export const BARO_MODES: ModeKey[] = ["altHold"];
/** Modes that use GPS (offered only when GPS is enabled). */
export const GPS_MODES: ModeKey[] = ["posHold", "gpsRescue"];
/** Modes with their own section on the Switches step (not in the generic "add" row). */
export const SPECIAL_MODES: ModeKey[] = ["prearm", "airMode"];

/** Stable names written to the model (and sent to config_plan / config_apply). */
export const MODES = {
  arm: "ARM",
  angle: "ANGLE",
  horizon: "HORIZON",
  beeper: "BEEPER",
  turtle: "FLIP OVER AFTER CRASH",
  altHold: "ALTHOLD",
  posHold: "POSHOLD",
  gpsRescue: "GPS RESCUE",
  prearm: "PREARM",
  airMode: "AIR MODE",
} as const satisfies Record<ModeKey, string>;
export type StableModeName = (typeof MODES)[ModeKey];

/** First firmware with the renamed boxes (display names only). */
export const MODE_RENAME_SINCE = "2025.12.0";

/**
 * Modes config::modes manages only from a given firmware: box ids 3 / 11 were BARO /
 * GPS HOLD before 2025.12 and are not managed there.
 */
export const MODE_MIN_FIRMWARE: Partial<Record<ModeKey, string>> = { altHold: MODE_RENAME_SINCE, posHold: MODE_RENAME_SINCE };

/** Box names as the firmware displays them (labels only, never stored). */
const DISPLAY_NAMES: Record<ModeKey, { legacy: string; current: string }> = {
  arm: { legacy: "ARM", current: "ARM" },
  angle: { legacy: "ANGLE", current: "ANGLE" },
  horizon: { legacy: "HORIZON", current: "HORIZON" },
  beeper: { legacy: "BEEPER", current: "BEEPER" },
  turtle: { legacy: "FLIP OVER AFTER CRASH", current: "FLIP OVER AFTER CRASH" },
  altHold: { legacy: "BARO", current: "ALTHOLD" },
  posHold: { legacy: "GPS HOLD", current: "POS HOLD" },
  gpsRescue: { legacy: "GPS RESCUE", current: "GPS RESCUE" },
  prearm: { legacy: "PREARM", current: "PREARM" },
  airMode: { legacy: "AIR MODE", current: "AIR MODE" },
};

export function usesNewModeNames(firmwareVersion: string | null | undefined): boolean {
  return !!firmwareVersion && compareVersions(firmwareVersion, MODE_RENAME_SINCE) >= 0;
}

/** Stable name of `key` (what the model stores). */
export function modeName(key: ModeKey): StableModeName {
  return MODES[key];
}

/** The firmware's own box name for `key` (display only; unknown version = 4.x names). */
export function displayBoxName(key: ModeKey, firmwareVersion: string | null | undefined): string {
  const n = DISPLAY_NAMES[key];
  return usesNewModeNames(firmwareVersion) ? n.current : n.legacy;
}

/**
 * Can the config engine set this mode on the given firmware? With the firmware catalog
 * (config_firmware_catalog().modes, non-empty) that list decides; otherwise the built-in
 * minimum versions (unknown version = yes).
 */
export function modeAvailable(key: ModeKey, firmwareVersion: string | null | undefined, catalogModes?: string[] | null): boolean {
  if (catalogModes && catalogModes.length) return catalogModes.some((m) => squash(m) === squash(MODES[key]));
  const min = MODE_MIN_FIRMWARE[key];
  return !min || !firmwareVersion || compareVersions(firmwareVersion, min) >= 0;
}

/** Older spellings accepted when reading a model (e.g. saved by an earlier build). */
const ALIASES: Record<string, ModeKey> = { BARO: "altHold", ALTITUDEHOLD: "altHold", GPSHOLD: "posHold", POSITIONHOLD: "posHold", TURTLE: "turtle", FLIP: "turtle" };

/** Mode key for a stable name, a firmware display name or a known alias. */
export function modeKeyOf(name: string): ModeKey | null {
  const k = squash(name);
  for (const m of MODE_KEYS) if (squash(MODES[m]) === k || squash(DISPLAY_NAMES[m].current) === k || squash(DISPLAY_NAMES[m].legacy) === k) return m;
  return ALIASES[k] ?? null;
}

/** Rewrite any known spelling to the stable name (the model never holds display names). */
export function normalizeModeNames<T extends { mode: string }>(switches: T[]): void {
  for (const sw of switches) {
    const k = modeKeyOf(sw.mode);
    if (k) sw.mode = MODES[k];
  }
}

export const RANGE_MIN = 900;
export const RANGE_MAX = 2100;
export const RANGE_STEP = 25;
/** AUX1..AUX12 */
export const MAX_AUX = 12;

/** Modes that only work with a DShot ESC protocol (turtle spins the motors backwards via DShot commands). */
export const NEEDS_DSHOT: ModeKey[] = ["turtle"];

/** A switch must move at least this far (µs) to count as "flipped" during detection. */
export const DETECT_THRESHOLD = 300;

/**
 * Live switch detection: compare an RC frame with the frame taken when the user pressed
 * "Detect", and return the AUX channel (0-based) that moved the most, with the position
 * it is in now. null = nothing moved far enough yet.
 */
export function detectSwitch(baseline: number[], current: number[]): { auxChannel: number; position: SwitchPosition } | null {
  let best: { aux: number; delta: number } | null = null;
  for (let aux = 0; aux < MAX_AUX; aux++) {
    const i = auxToChannelIndex(aux);
    const a = baseline[i];
    const b = current[i];
    if (a === undefined || b === undefined) continue;
    const d = Math.abs(b - a);
    if (d >= DETECT_THRESHOLD && (!best || d > best.delta)) best = { aux, delta: d };
  }
  if (!best) return null;
  const v = current[auxToChannelIndex(best.aux)]!;
  const position = (Object.keys(SWITCH_POSITIONS) as SwitchPosition[]).find((p) => inRange(v, SWITCH_POSITIONS[p][0], SWITCH_POSITIONS[p][1])) ?? (v < 1300 ? "low" : "high");
  return { auxChannel: best.aux, position };
}
