// Data-driven step engine: loads step definitions from steps.json and decides
// which steps are visible for the current DroneModel. Pure - unit tested.

import type { DroneModel } from "../types/model";
import stepsFile from "./steps.json";

export interface StepCondition {
  path: string;
  equals?: unknown;
  notEquals?: unknown;
  in?: unknown[];
}

export interface StepDef {
  id: string;
  titleKey: string;
  component: string;
  when?: StepCondition[];
}

export const ALL_STEPS: StepDef[] = (stepsFile as { steps: StepDef[] }).steps;

export function getPath(obj: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>(
    (acc, key) => (acc && typeof acc === "object" ? (acc as Record<string, unknown>)[key] : undefined),
    obj,
  );
}

export function conditionHolds(model: DroneModel, c: StepCondition): boolean {
  const v = getPath(model, c.path);
  if ("equals" in c && v !== c.equals) return false;
  if ("notEquals" in c && v === c.notEquals) return false;
  if (c.in && !c.in.includes(v)) return false;
  return true;
}

export function visibleSteps(model: DroneModel, steps: StepDef[] = ALL_STEPS): StepDef[] {
  return steps.filter((s) => (s.when ?? []).every((c) => conditionHolds(model, c)));
}
