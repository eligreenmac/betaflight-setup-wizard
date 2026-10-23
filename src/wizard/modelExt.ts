// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UI-side adapter for the DroneModel batch-2 fields of shared PR #65 (airmode,
 * removeModes, sensors, boardAlignment, beepers; the OSD is in its own PR). The exact #65
 * types are mirrored in src/types/pendingModel.ts (TODO(shared): import them from
 * src/types/model.ts when #65 lands; the shared file is not edited here).
 *
 * Like #65: everything optional, null = keep the flight controller's current value.
 * Untouched answers stay null; only an explicit choice sets a value. `sensors` is per
 * sensor: only the toggles the user touched are sent.
 *
 * An engine that does not know these fields yet ignores them (serde defaults); Review
 * detects that with unplannedChoices() and says so instead of pretending they were written.
 */
import type { CliScript, DroneModel } from "../types/model";
import type { Airmode, Beepers, BoardAlignment, DroneModelNext, OptionalMode } from "../types/pendingModel";
import { sourceMismatch } from "../api/fallback";
import { SENSORS_COMMAND, ATTITUDE_START_COMMAND } from "../api/pending";

export type { Airmode, Beepers, BoardAlignment, OptionalMode };
export interface SensorsChoice {
  acc: boolean | null;
  baro: boolean | null;
  mag: boolean | null;
}

export interface ModelExt {
  airmode: Airmode | null;
  /** Optional modes whose existing FC ranges are deleted ([] = none). */
  removeModes: OptionalMode[];
  sensors: SensorsChoice | null;
  boardAlignment: BoardAlignment | null;
  beepers: Beepers | null;
}

export const emptyExt = (): ModelExt => ({ airmode: null, removeModes: [], sensors: null, boardAlignment: null, beepers: null });

/** DroneModel plus the #65 fields (what config_plan / config_verify receive). */
export type ExtendedDroneModel = DroneModelNext;

export type ExtField = keyof ModelExt;
export const EXT_FIELDS: ExtField[] = ["airmode", "removeModes", "sensors", "boardAlignment", "beepers"];

/** Is this answer set (anything other than keep)? */
function isSet(ext: ModelExt, f: ExtField): boolean {
  if (f === "removeModes") return ext.removeModes.length > 0;
  if (f === "sensors") return !!ext.sensors && Object.values(ext.sensors).some((v) => v !== null);
  return ext[f] !== null;
}

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

/** The model to plan / verify: shared fields + every explicit (non-null) #65 answer. */
export function toPlanModel(model: DroneModel, ext: ModelExt): ExtendedDroneModel {
  const out: ExtendedDroneModel = JSON.parse(JSON.stringify(model));
  const skip = new Set(simulatedFields());
  for (const f of EXT_FIELDS) {
    if (!isSet(ext, f) || skip.has(f)) continue;
    let v: unknown = JSON.parse(JSON.stringify(ext[f]));
    if (f === "sensors") v = Object.fromEntries(Object.entries(v as SensorsChoice).filter(([, x]) => x !== null));
    (out as unknown as Record<string, unknown>)[f] = v;
  }
  return out;
}

/** CLI fragments that show a plan really writes an answer (BF CLI names). */
const MARKERS: Record<ExtField, RegExp> = {
  airmode: /^feature -?AIRMODE\b/,
  removeModes: /^aux \d+ /,
  sensors: /^set (acc_hardware|baro_hardware|mag_hardware)\b/,
  boardAlignment: /^set align_board_(roll|pitch|yaw)\b/,
  beepers: /^(beeper|beacon) -?[A-Z_]+/,
};

/**
 * Explicit answers the plan does not contain: the running config engine does not know
 * the field yet. Review lists them ("not written by this version") so nobody assumes
 * they were applied.
 */
export function unplannedChoices(ext: ModelExt, script: CliScript | null): ExtField[] {
  if (!script) return [];
  const skip = new Set(simulatedFields());
  return EXT_FIELDS.filter((f) => isSet(ext, f) && !skip.has(f) && !script.commands.some((c) => MARKERS[f].test(c)));
}
