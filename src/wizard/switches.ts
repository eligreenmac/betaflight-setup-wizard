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

/**
 * Modes the wizard offers. DroneModel.switches[].mode stores the box NAME used by the
 * connected firmware; the config engine maps names to box ids per dialect.
 * Firmware 2025.12 renamed some boxes, so names (and labels) depend on the version.
 * TODO(shared): confirm the mode vocabulary with the `config` workstream.
 */
export const MODE_KEYS = ["arm", "angle", "beeper", "turtle", "altHold", "posHold"] as const;
export type ModeKey = (typeof MODE_KEYS)[number];
export const OPTIONAL_MODES: ModeKey[] = ["angle", "beeper", "turtle", "altHold", "posHold"];

/** First firmware with the renamed boxes. */
export const MODE_RENAME_SINCE = "2025.12.0";

const MODE_NAMES: Record<ModeKey, { legacy: string; current: string }> = {
  arm: { legacy: "ARM", current: "ARM" },
  angle: { legacy: "ANGLE", current: "ANGLE" },
  beeper: { legacy: "BEEPER", current: "BEEPER" },
  turtle: { legacy: "FLIP OVER AFTER CRASH", current: "FLIP OVER AFTER CRASH" },
  altHold: { legacy: "BARO", current: "ALTHOLD" },
  posHold: { legacy: "GPS HOLD", current: "POS HOLD" },
};

/** Version-independent names (same on every supported firmware). */
export const MODES = { arm: "ARM", angle: "ANGLE", beeper: "BEEPER", turtle: "FLIP OVER AFTER CRASH" } as const;

export function usesNewModeNames(firmwareVersion: string | null | undefined): boolean {
  return !!firmwareVersion && compareVersions(firmwareVersion, MODE_RENAME_SINCE) >= 0;
}

/** Box name of `key` on the given firmware (unknown version = 4.x names). */
export function modeName(key: ModeKey, firmwareVersion: string | null | undefined): string {
  const n = MODE_NAMES[key];
  return usesNewModeNames(firmwareVersion) ? n.current : n.legacy;
}

/** Mode key for a box name from either naming generation. */
export function modeKeyOf(name: string): ModeKey | null {
  for (const k of MODE_KEYS) if (MODE_NAMES[k].legacy === name || MODE_NAMES[k].current === name) return k;
  return null;
}

/** Rewrite stored mode names after the firmware version changed (e.g. after flashing). */
export function renameModesForFirmware<T extends { mode: string }>(switches: T[], firmwareVersion: string): void {
  for (const sw of switches) {
    const k = modeKeyOf(sw.mode);
    if (k) sw.mode = modeName(k, firmwareVersion);
  }
}

export const RANGE_MIN = 900;
export const RANGE_MAX = 2100;
export const RANGE_STEP = 25;
/** AUX1..AUX12 */
export const MAX_AUX = 12;
