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

/**
 * Mode names written to DroneModel.switches[].mode. Betaflight box names; the config
 * engine maps them to box ids per firmware dialect.
 * TODO(shared): confirm the mode vocabulary with the `config` workstream.
 */
export const MODES = {
  arm: "ARM",
  angle: "ANGLE",
  beeper: "BEEPER",
  turtle: "FLIP OVER AFTER CRASH",
} as const;
export type ModeKey = keyof typeof MODES;
export const OPTIONAL_MODES: ModeKey[] = ["angle", "beeper", "turtle"];

export const RANGE_MIN = 900;
export const RANGE_MAX = 2100;
export const RANGE_STEP = 25;
/** AUX1..AUX12 */
export const MAX_AUX = 12;
