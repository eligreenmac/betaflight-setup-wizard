// SPDX-License-Identifier: GPL-3.0-or-later
// Maps `component` keys in steps.json to Vue components.

import type { Component } from "vue";
import ApplyStep from "./steps/ApplyStep.vue";
import BindingStep from "./steps/BindingStep.vue";
import ComponentsStep from "./steps/ComponentsStep.vue";
import ConnectStep from "./steps/ConnectStep.vue";
import DoneStep from "./steps/DoneStep.vue";
import EscStep from "./steps/EscStep.vue";
import FirmwareStep from "./steps/FirmwareStep.vue";
import GpsStep from "./steps/GpsStep.vue";
import ReceiverStep from "./steps/ReceiverStep.vue";
import ReviewStep from "./steps/ReviewStep.vue";
import OsdStep from "./steps/OsdStep.vue";
import SwitchesStep from "./steps/SwitchesStep.vue";
import VtxStep from "./steps/VtxStep.vue";
import WelcomeStep from "./steps/WelcomeStep.vue";

export const stepComponents: Record<string, Component> = {
  Welcome: WelcomeStep,
  Connect: ConnectStep,
  Firmware: FirmwareStep,
  Components: ComponentsStep,
  Receiver: ReceiverStep,
  Binding: BindingStep,
  Vtx: VtxStep,
  Gps: GpsStep,
  Esc: EscStep,
  Switches: SwitchesStep,
  Osd: OsdStep,
  Review: ReviewStep,
  Apply: ApplyStep,
  Done: DoneStep,
};

export function resolveStepComponent(key: string): Component {
  const c = stepComponents[key];
  if (!c) throw new Error(`Unknown wizard step component: ${key}`);
  return c;
}
