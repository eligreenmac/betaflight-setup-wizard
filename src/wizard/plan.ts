// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Review plan: what apply will do, with the managed-role clearings first.
 * Prefers the backend's read-only config_plan; until that command exists
 * (TODO(shared)) falls back to config_generate + the wizard's own clearing list.
 */
import { api } from "../api/commands";
import { upcoming, type ConfigPlan, type PlanRole } from "../api/upcoming";
import { isNotImplemented } from "../errors/guidance";
import type { DroneModel, FcSerialPort } from "../types/model";
import { plannedClearings, type PortRole, type RoleOptions } from "./ports";

export const ROLE_TO_PLAN: Record<PortRole, PlanRole> = { receiver: "RX", vtx: "VTX", gps: "GPS", escTelemetry: "ESC_TELEMETRY" };

export interface PlanResult {
  plan: ConfigPlan;
  /** "backend" = config_plan; "wizard" = UI fallback (clearings computed in the UI). */
  source: "backend" | "wizard";
}

export async function fetchPlan(model: DroneModel, ports: FcSerialPort[], opts: RoleOptions): Promise<PlanResult> {
  try {
    return { plan: await upcoming.configPlan(model, ports), source: "backend" };
  } catch (e) {
    if (!isNotImplemented(e)) throw e;
  }
  const script = await api.config.generate(model, ports);
  const clearings = plannedClearings(ports, model, opts).map((c) => ({ port: c.port, roles: c.roles.map((r) => ROLE_TO_PLAN[r]) }));
  return { plan: { ...script, clearings }, source: "wizard" };
}
