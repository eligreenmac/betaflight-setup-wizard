// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Review plan (ARCHITECTURE §5.1): config_plan is read-only and its script is the
 * ONLY one that may be passed to config_apply. config_generate is a preview: it is
 * shown when the running backend has no config_plan yet, but Apply stays blocked.
 */
import { api } from "../api/commands";
import { isNotImplemented } from "../errors/codes";
import type { CliScript, ConfigStage, DroneModel, FcSerialPort, SerialFunction } from "../types/model";

/** `config://progress` stages, in the order config_apply emits them. */
export const APPLY_STAGES: readonly ConfigStage[] = ["BACKUP", "SENDING", "SAVING", "REBOOTING", "RECONNECTED"];

/** Serial functions a plan can clear (labels: `serialFunctions.*`). */
export const SERIAL_FUNCTIONS: readonly SerialFunction[] = ["RX_SERIAL", "GPS", "ESC_SENSOR", "VTX_SMARTAUDIO", "VTX_TRAMP", "VTX_MSP", "MSP"];

export type PlanResult =
  /** config_plan answered: `script` is what Apply sends, unchanged. */
  | { status: "ready"; script: CliScript }
  /** Backend without config_plan: preview only (may be null), Apply blocked. */
  | { status: "unavailable"; preview: CliScript | null };

export async function fetchPlan(model: DroneModel, ports: FcSerialPort[]): Promise<PlanResult> {
  try {
    return { status: "ready", script: await api.config.plan(model) };
  } catch (e) {
    if (!isNotImplemented(e)) throw e;
  }
  // Preview only; never applied.
  try {
    return { status: "unavailable", preview: await api.config.generate(model, ports) };
  } catch {
    return { status: "unavailable", preview: null };
  }
}
