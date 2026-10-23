// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Beeper conditions (Betaflight io/beeper.c `beeperTable`). The model stores the
 * firmware's own names in `beepers.disabled` (BF `beeper -NAME`).
 *
 * Availability per firmware line (checked against the BF sources):
 *   4.4 / 4.5: CRASH_FLIP, RC_SMOOTHING_INIT_FAIL
 *   2025.12  : CRASH_FLIP is called CRASHFLIP; RC_SMOOTHING_INIT_FAIL is gone.
 * MULTI_BEEPS and ALL are internal / pseudo entries and are never offered.
 * TODO(shared): Config will publish availability in the contract; until then this table
 * is used. Unknown firmware (demo mode) shows the full list.
 */
import { compareVersions } from "./version";

export type BeeperGroup = "important" | "helpful" | "noisy";
export interface BeeperDef {
  /** UI key (i18n `beepers.items.<key>`). */
  key: string;
  group: BeeperGroup;
  /** Firmware name on 4.4 / 4.5. */
  name: string;
  /** Firmware name from 2025.12 if different. */
  name2025?: string;
  /** Not available from 2025.12. */
  removedIn2025?: boolean;
}

export const BEEPERS: BeeperDef[] = [
  { key: "RX_LOST", group: "important", name: "RX_LOST" },
  { key: "RX_LOST_LANDING", group: "important", name: "RX_LOST_LANDING" },
  { key: "BAT_LOW", group: "important", name: "BAT_LOW" },
  { key: "BAT_CRIT_LOW", group: "important", name: "BAT_CRIT_LOW" },
  { key: "RX_SET", group: "important", name: "RX_SET" },
  { key: "ARMING", group: "important", name: "ARMING" },
  { key: "DISARMING", group: "important", name: "DISARMING" },
  { key: "ARMED", group: "helpful", name: "ARMED" },
  { key: "GYRO_CALIBRATED", group: "helpful", name: "GYRO_CALIBRATED" },
  { key: "ACC_CALIBRATION", group: "helpful", name: "ACC_CALIBRATION" },
  { key: "ACC_CALIBRATION_FAIL", group: "helpful", name: "ACC_CALIBRATION_FAIL" },
  { key: "ARMING_GPS_FIX", group: "helpful", name: "ARMING_GPS_FIX" },
  { key: "ARMING_GPS_NO_FIX", group: "helpful", name: "ARMING_GPS_NO_FIX" },
  { key: "GPS_STATUS", group: "helpful", name: "GPS_STATUS" },
  { key: "READY_BEEP", group: "helpful", name: "READY_BEEP" },
  { key: "CRASH_FLIP", group: "helpful", name: "CRASH_FLIP", name2025: "CRASHFLIP" },
  { key: "BLACKBOX_ERASE", group: "helpful", name: "BLACKBOX_ERASE" },
  { key: "RC_SMOOTHING_INIT_FAIL", group: "helpful", name: "RC_SMOOTHING_INIT_FAIL", removedIn2025: true },
  { key: "SYSTEM_INIT", group: "noisy", name: "SYSTEM_INIT" },
  { key: "ON_USB", group: "noisy", name: "ON_USB" },
  { key: "DISARM_REPEAT", group: "noisy", name: "DISARM_REPEAT" },
  { key: "CAM_CONNECTION_OPEN", group: "noisy", name: "CAM_CONNECTION_OPEN" },
  { key: "CAM_CONNECTION_CLOSE", group: "noisy", name: "CAM_CONNECTION_CLOSE" },
];
export const BEEPER_GROUPS: BeeperGroup[] = ["important", "helpful", "noisy"];

/** Betaflight's default is every beeper on (beeper_off_flags = 0). We only turn off ON_USB. */
export const RECOMMENDED_OFF = ["ON_USB"];
/** Never recommended off: losing these can cost the quad (lost signal, flat battery, finding it). */
export const KEEP_ON = ["RX_LOST", "RX_LOST_LANDING", "BAT_LOW", "BAT_CRIT_LOW"];

const is2025 = (fw: string | null | undefined) => !!fw && compareVersions(fw, "2025.12.0") >= 0;

/** Beepers offered for this firmware. null/empty version (demo, unknown) = the full list. */
export function availableBeepers(fw: string | null | undefined): BeeperDef[] {
  if (!fw) return BEEPERS;
  return BEEPERS.filter((b) => !(is2025(fw) && b.removedIn2025));
}

/** The firmware's name for a beeper. */
export function beeperName(b: BeeperDef, fw: string | null | undefined): string {
  return is2025(fw) && b.name2025 ? b.name2025 : b.name;
}

/** Recommended `beepers.disabled` for this firmware. */
export function recommendedDisabled(fw: string | null | undefined): string[] {
  return availableBeepers(fw).filter((b) => RECOMMENDED_OFF.includes(b.key)).map((b) => beeperName(b, fw));
}

export function isBeeperOn(disabled: string[], b: BeeperDef, fw: string | null | undefined): boolean {
  return !disabled.includes(beeperName(b, fw));
}

/** Toggle one beeper in a disabled list (returns a new, firmware-ordered list). */
export function setBeeper(disabled: string[], b: BeeperDef, on: boolean, fw: string | null | undefined): string[] {
  const n = beeperName(b, fw);
  const set = new Set(disabled);
  if (on) set.delete(n);
  else set.add(n);
  return availableBeepers(fw).map((x) => beeperName(x, fw)).filter((x) => set.has(x));
}
