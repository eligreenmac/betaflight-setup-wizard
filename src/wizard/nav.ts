// SPDX-License-Identifier: GPL-3.0-or-later
// Navigation over the VISIBLE steps (engine.visibleSteps). Tracks the current step by
// id, so toggling a component on/off (which shows/hides steps) never jumps the user.
import { computed, reactive } from "vue";
import { visibleSteps, type StepDef } from "./engine";
import { canGoTo, canLeave, guardFor } from "./guards";
import { wizard } from "./state";

export const nav = reactive({ currentId: "welcome", attemptedNext: false });

export const steps = computed<StepDef[]>(() => visibleSteps(wizard.model));
export const currentIndex = computed(() => {
  const i = steps.value.findIndex((s) => s.id === nav.currentId);
  return i < 0 ? 0 : i;
});
export const currentStep = computed<StepDef>(() => steps.value[currentIndex.value]!);
export const isFirst = computed(() => currentIndex.value === 0);
export const isLast = computed(() => currentIndex.value === steps.value.length - 1);
/** 0..100: share of steps completed. */
export const progress = computed(() => Math.round((currentIndex.value / Math.max(1, steps.value.length - 1)) * 100));
export const issues = computed(() => guardFor(currentStep.value.id).validate(wizard));

export function canNext(): boolean {
  if (isLast.value) return false;
  const next = steps.value[currentIndex.value + 1]!;
  return canLeave(currentStep.value.id, wizard) && guardFor(next.id).canEnter(wizard);
}

export function next(): boolean {
  nav.attemptedNext = true;
  if (!canNext()) return false;
  nav.currentId = steps.value[currentIndex.value + 1]!.id;
  nav.attemptedNext = false;
  return true;
}

export function back(): void {
  if (!isFirst.value) nav.currentId = steps.value[currentIndex.value - 1]!.id;
  nav.attemptedNext = false;
}

export function goTo(id: string): boolean {
  if (!canGoTo(id, currentStep.value.id, wizard, steps.value)) return false;
  nav.currentId = id;
  nav.attemptedNext = false;
  return true;
}

export function resetNav(): void {
  nav.currentId = "welcome";
  nav.attemptedNext = false;
}
