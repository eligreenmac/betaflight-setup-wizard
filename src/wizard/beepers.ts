// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Beeper conditions (Betaflight io/beeper.c `beeperTable`). The model stores the #65
 * names (BeeperName, the 4.4/4.5 CLI spelling) in `beepers.disabled`, as the FULL state:
 * listed = `beeper -X`, every other condition the firmware has = `beeper X`. The config
 * engine translates per firmware (CRASH_FLIP is written as CRASHFLIP on 2025.12 and 2026.6).
 *
 * Which conditions the firmware has comes from config_firmware_catalog().beepers (see
 * api/catalog.ts). Without a catalog (not loaded / empty) the full #65 list is shown.
 */
import { catalogHas } from "../api/catalog";
import { BEEPER_NAMES, type BeeperName, type DshotBeaconCondition } from "../types/model";

export type BeeperGroup = "important" | "helpful" | "noisy";
export interface BeeperDef {
  /** #65 BeeperName (also the i18n key `beepers.items.<name>`). */
  name: BeeperName;
  group: BeeperGroup;
}

const GROUP: Record<BeeperName, BeeperGroup> = {
  RX_LOST: "important",
  RX_LOST_LANDING: "important",
  BAT_LOW: "important",
  BAT_CRIT_LOW: "important",
  RX_SET: "important",
  ARMING: "important",
  DISARMING: "important",
  ARMED: "helpful",
  GYRO_CALIBRATED: "helpful",
  ACC_CALIBRATION: "helpful",
  ACC_CALIBRATION_FAIL: "helpful",
  ARMING_GPS_FIX: "helpful",
  ARMING_GPS_NO_FIX: "helpful",
  GPS_STATUS: "helpful",
  READY_BEEP: "helpful",
  MULTI_BEEPS: "helpful",
  CRASH_FLIP: "helpful",
  BLACKBOX_ERASE: "helpful",
  RC_SMOOTHING_INIT_FAIL: "helpful",
  SYSTEM_INIT: "noisy",
  ON_USB: "noisy",
  DISARM_REPEAT: "noisy",
  CAM_CONNECTION_OPEN: "noisy",
  CAM_CONNECTION_CLOSE: "noisy",
};
const ORDER = Object.keys(GROUP) as BeeperName[];

/** Every #65 beeper, in display order. */
export const BEEPERS: BeeperDef[] = ORDER.filter((n) => (BEEPER_NAMES as readonly string[]).includes(n)).map((name) => ({ name, group: GROUP[name] }));
export const BEEPER_GROUPS: BeeperGroup[] = ["important", "helpful", "noisy"];

/** Betaflight's default is every beeper on. We only turn off ON_USB (#65 default {disabled:['ON_USB']}). */
export const RECOMMENDED_OFF: BeeperName[] = ["ON_USB"];
/** Never recommended off: losing these can cost the quad (lost signal, flat battery, finding it). */
export const KEEP_ON: BeeperName[] = ["RX_LOST", "RX_LOST_LANDING", "BAT_LOW", "BAT_CRIT_LOW"];

/** Beepers this firmware has (catalog list; null/empty = the full list). */
export function availableBeepers(catalog: string[] | null | undefined): BeeperDef[] {
  if (!catalog || !catalog.length) return BEEPERS;
  return BEEPERS.filter((b) => catalogHas(catalog, b.name));
}

/** Recommended `beepers.disabled` for this firmware. */
export function recommendedDisabled(catalog: string[] | null | undefined): BeeperName[] {
  return availableBeepers(catalog).filter((b) => RECOMMENDED_OFF.includes(b.name)).map((b) => b.name);
}

export const isBeeperOn = (disabled: readonly string[], b: BeeperDef): boolean => !disabled.includes(b.name);

/** Toggle one beeper in a disabled list (returns a new list in display order, only names the firmware has). */
export function setBeeper(disabled: readonly string[], b: BeeperDef, on: boolean, catalog: string[] | null | undefined): BeeperName[] {
  const set = new Set(disabled);
  if (on) set.delete(b.name);
  else set.add(b.name);
  return availableBeepers(catalog).map((x) => x.name).filter((x) => set.has(x));
}

/** DShot beacon events (#65 DshotBeaconCondition), from the catalog's beaconEvents. */
export const BEACON_EVENTS: DshotBeaconCondition[] = ["RX_LOST", "RX_SET"];
export function availableBeaconEvents(catalog: string[] | null | undefined): DshotBeaconCondition[] {
  if (!catalog || !catalog.length) return BEACON_EVENTS;
  return BEACON_EVENTS.filter((e) => catalogHas(catalog, e));
}
/** Toggle one beacon event (on = beacon sounds for it). */
export function setBeacon(disabled: readonly DshotBeaconCondition[], e: DshotBeaconCondition, on: boolean): DshotBeaconCondition[] {
  const set = new Set(disabled);
  if (on) set.delete(e);
  else set.add(e);
  return BEACON_EVENTS.filter((x) => set.has(x));
}
