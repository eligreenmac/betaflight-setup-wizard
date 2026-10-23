// Maps `component` keys in steps.json to Vue components.
// The `ui` workstream replaces placeholders with real step components.

import type { Component } from "vue";
import PlaceholderStep from "./steps/PlaceholderStep.vue";
import WelcomeStep from "./steps/WelcomeStep.vue";

export const stepComponents: Record<string, Component> = {
  Welcome: WelcomeStep,
  Connect: PlaceholderStep,
  Firmware: PlaceholderStep,
  Components: PlaceholderStep,
  Receiver: PlaceholderStep,
  Vtx: PlaceholderStep,
  Gps: PlaceholderStep,
  Esc: PlaceholderStep,
  Switches: PlaceholderStep,
  Review: PlaceholderStep,
  Apply: PlaceholderStep,
  Done: PlaceholderStep,
};

export function resolveStepComponent(key: string): Component {
  const c = stepComponents[key];
  if (!c) throw new Error(`Unknown wizard step component: ${key}`);
  return c;
}
