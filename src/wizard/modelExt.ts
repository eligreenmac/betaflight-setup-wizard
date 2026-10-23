// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UI-side adapter for DroneModel fields that are proposed to Config Engine Dev but not
 * in the shared contract yet (src/types/model.ts is a shared file; it is NOT edited here).
 *
 * TODO(shared): once Config's `shared` PR lands, move these types to src/types/model.ts
 * (and drop this adapter's casts). Proposed shape:
 *   airmode: "always" | "switch" | "off" | null
 *   sensors: { acc: boolean, baro: boolean } | null
 *   boardAlignment: { roll, pitch, yaw } | null      (degrees)
 *   beepers: { disabled: BeeperName[] } | null       (firmware beeper names)
 * Like #53: null = keep the flight controller's current value. Untouched answers stay
 * null; only an explicit choice on the Switches / Calibration / Beepers steps sets a value.
 *
 * The backend deserialises DroneModel with serde defaults (no deny_unknown_fields), so
 * an engine that does not know these fields yet ignores them. Review detects that with
 * unplannedChoices() and says so instead of pretending they were written.
 */
import type { CliScript, DroneModel } from "../types/model";
import { sourceMismatch } from "../api/fallback";
import { SENSORS_COMMAND, ATTITUDE_START_COMMAND } from "../api/pending";

export type Airmode = "always" | "switch" | "off";
export interface SensorsChoice {
  acc: boolean;
  baro: boolean;
}
export interface BoardAlignment {
  roll: number;
  pitch: number;
  yaw: number;
}
export interface BeepersChoice {
  disabled: string[];
}

export interface ModelExt {
  airmode: Airmode | null;
  sensors: SensorsChoice | null;
  boardAlignment: BoardAlignment | null;
  beepers: BeepersChoice | null;
}

export const emptyExt = (): ModelExt => ({ airmode: null, sensors: null, boardAlignment: null, beepers: null });

/** DroneModel plus the proposed fields (what config_plan / config_verify receive). */
export type ExtendedDroneModel = DroneModel & Partial<ModelExt>;

export type ExtField = keyof ModelExt;
export const EXT_FIELDS: ExtField[] = ["airmode", "sensors", "boardAlignment", "beepers"];

/**
 * Fields whose answers were based on simulated board data while the board is real
 * (e.g. the sensor list or the attitude stream came from the demo fallback). Such
 * answers are never sent: the field stays null (= keep the board's value).
 */
export function simulatedFields(): ExtField[] {
  const out: ExtField[] = [];
  if (sourceMismatch(SENSORS_COMMAND)) out.push("sensors");
  if (sourceMismatch(ATTITUDE_START_COMMAND)) out.push("boardAlignment");
  return out;
}

/** The model to plan / verify: shared fields + every explicit (non-null) extension answer. */
export function toPlanModel(model: DroneModel, ext: ModelExt): ExtendedDroneModel {
  const out: ExtendedDroneModel = JSON.parse(JSON.stringify(model));
  const skip = new Set(simulatedFields());
  for (const f of EXT_FIELDS) {
    const v = ext[f];
    if (v !== null && !skip.has(f)) (out as unknown as Record<string, unknown>)[f] = JSON.parse(JSON.stringify(v));
  }
  return out;
}

/** CLI fragments that show a plan really writes an answer (BF CLI names). */
const MARKERS: Record<ExtField, RegExp> = {
  airmode: /^feature -?AIRMODE\b/,
  sensors: /^set (acc_hardware|baro_hardware)\b/,
  boardAlignment: /^set align_board_(roll|pitch|yaw)\b/,
  beepers: /^beeper -?[A-Z_]+/,
};

/**
 * Explicit answers the plan does not contain: the running config engine does not know
 * the field yet. Review lists them ("not written by this version") so nobody assumes
 * they were applied.
 */
export function unplannedChoices(ext: ModelExt, script: CliScript | null): ExtField[] {
  if (!script) return [];
  const skip = new Set(simulatedFields());
  return EXT_FIELDS.filter((f) => ext[f] !== null && !skip.has(f) && !script.commands.some((c) => MARKERS[f].test(c)));
}
