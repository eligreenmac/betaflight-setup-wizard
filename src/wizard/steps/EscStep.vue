<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U5: ESC protocol / bidirectional DShot / motor poles / props-out, ESC telemetry UART,
// and the safety-gated motor test (src/wizard/motorTest.ts).
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api } from "../../api/commands";
import ErrorPanel from "../../components/ErrorPanel.vue";
import PortPicker from "../../components/PortPicker.vue";
import { isNotImplemented } from "../../errors/codes";
import type { EscProtocol } from "../../types/model";
import { acksComplete, motorTest } from "../motorTest";
import { neededRoles } from "../ports";
import { markDirty, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const esc = computed(() => wizard.model.esc);
const protocols: EscProtocol[] = ["DSHOT600", "DSHOT300", "DSHOT150", "ONESHOT125", "MULTISHOT", "PWM"];
const isDshot = computed(() => esc.value.protocol.startsWith("DSHOT"));
const acks = computed(() => ({ propsRemoved: wizard.ui.propsOffAck, batteryConnected: wizard.ui.batteryOnAck }));
const canTest = computed(() => wizard.ui.motorTestAvailable === true && acksComplete(acks.value));
const needsTelemetryPort = computed(() => neededRoles(wizard.model, wizard.ui).includes("escTelemetry"));

/**
 * Betaflight Quad X, seen from above, nose up. Physical layout: rendered dir="ltr"
 * so it never mirrors in Hebrew. Default (props in): M1 CW, M2 CCW, M3 CCW, M4 CW.
 */
const layout = [
  { motor: 3, label: "M4", cwPropsIn: true },
  { motor: 1, label: "M2", cwPropsIn: false },
  { motor: 2, label: "M3", cwPropsIn: false },
  { motor: 0, label: "M1", cwPropsIn: true },
];
const cw = (m: { cwPropsIn: boolean }) => (esc.value.propsOut ? !m.cwPropsIn : m.cwPropsIn);

function set<K extends keyof typeof esc.value>(k: K, v: (typeof esc.value)[K]): void {
  wizard.model.esc[k] = v;
  if (k === "protocol" && !String(v).startsWith("DSHOT")) wizard.model.esc.bidirectionalDshot = false;
  markDirty();
}

/** A motor test was started on this visit (the backend accepted it). */
const started = ref(false);
/**
 * No backend signal tells us whether the ESCs are powered (Betaflight reports no
 * spin feedback without telemetry), so after a start without the "right motor spins"
 * confirmation, show the "Motors not spinning?" hint next to the controls.
 */
const showNoSpinHint = computed(() => started.value && !wizard.ui.motorOrderOk && !wizard.ui.motorDirectionOk);

async function spin(motor: number): Promise<void> {
  if (motorTest.state.running && motorTest.state.motor === motor) return void motorTest.stop("requested");
  try {
    await motorTest.start(motor, { port: wizard.connectedPath, acks: acks.value });
    started.value = true;
  } catch {
    /* shown via motorTest.state.error */
  }
}

// Withdrawing an acknowledgement stops the motors immediately.
watch(acks, (a) => {
  if (!acksComplete(a)) void motorTest.stop("acks-withdrawn");
});

onMounted(async () => {
  if (wizard.ui.motorTestAvailable !== null) return;
  // Probe: device_motor_test_stop is always safe to send (a no-op when idle).
  try {
    await api.device.motorTestStop();
    wizard.ui.motorTestAvailable = true;
  } catch (e) {
    wizard.ui.motorTestAvailable = !isNotImplemented(e);
  }
});
onBeforeUnmount(() => void motorTest.stop("unmount"));
</script>

<template>
  <div>
    <h2>{{ t("steps.esc.title") }}</h2>
    <p class="lead">{{ t("steps.esc.description") }}</p>

    <div class="field">
      <label for="esc-protocol">{{ t("esc.protocol") }}</label>
      <select id="esc-protocol" :value="esc.protocol" @change="set('protocol', ($event.target as HTMLSelectElement).value as EscProtocol)">
        <option v-for="p in protocols" :key="p" :value="p">{{ t(`esc.protocols.${p}`) }}</option>
      </select>
      <p class="hint">{{ t("esc.protocolHint") }}</p>
    </div>

    <label class="choice">
      <input type="checkbox" :checked="esc.bidirectionalDshot" :disabled="!isDshot" @change="set('bidirectionalDshot', ($event.target as HTMLInputElement).checked)" />
      <span>{{ t("esc.bidir") }}<small>{{ t("esc.bidirHint") }}</small></span>
    </label>
    <div v-if="esc.bidirectionalDshot" class="field">
      <label for="poles">{{ t("esc.poles") }}</label>
      <input id="poles" type="number" min="2" max="64" step="2" :value="esc.motorPoles ?? ''" @input="set('motorPoles', Number(($event.target as HTMLInputElement).value) || null)" />
      <p class="hint">{{ t("esc.polesHint") }}</p>
    </div>

    <label class="choice">
      <input type="checkbox" :checked="esc.propsOut" @change="set('propsOut', ($event.target as HTMLInputElement).checked)" />
      <span>{{ t("esc.propsOut") }}<small>{{ t("esc.propsOutHint") }}</small></span>
    </label>

    <PortPicker v-if="needsTelemetryPort" role="escTelemetry" />

    <fieldset class="disabled-stub" aria-disabled="true">
      <legend>{{ t("esc.reverse") }} <span class="badge">{{ t("common.phase2") }}</span></legend>
      <label class="choice"><input type="checkbox" disabled /> {{ t("esc.reverseHint") }}</label>
    </fieldset>

    <h3>{{ t("esc.motorTest.title") }}</h3>
    <div v-if="wizard.ui.motorTestAvailable === false" class="callout callout--warn" data-test="motor-test-unavailable">
      <p>{{ t("esc.motorTest.unavailable") }}</p>
    </div>
    <template v-else>
      <div class="callout callout--danger">
        <p><strong>{{ t("esc.motorTest.warning") }}</strong></p>
        <p data-test="motor-safety">{{ t("esc.motorTest.keepClear") }}</p>
        <label class="choice">
          <input v-model="wizard.ui.propsOffAck" type="checkbox" data-test="ack-props" />
          <span>{{ t("esc.motorTest.ackProps") }}</span>
        </label>
        <label class="choice">
          <input v-model="wizard.ui.batteryOnAck" type="checkbox" data-test="ack-battery" />
          <span>{{ t("esc.motorTest.ackBattery") }}<small>{{ t("esc.motorTest.ackBatteryHint") }}</small></span>
        </label>
      </div>

      <div class="quad" dir="ltr" :aria-label="t('esc.motorTest.diagram')" role="group">
        <div class="quad__front">▲ {{ t("esc.motorTest.front") }}</div>
        <button
          v-for="m in layout"
          :key="m.motor"
          type="button"
          class="motor"
          :disabled="!canTest"
          :aria-pressed="motorTest.state.running && motorTest.state.motor === m.motor"
          :data-motor="m.motor"
          :aria-label="t('esc.motorTest.spinMotor', { n: m.motor + 1, dir: t(cw(m) ? 'esc.motorTest.cw' : 'esc.motorTest.ccw') })"
          @click="spin(m.motor)"
        >
          <span>{{ m.label }}</span>
          <span class="motor__dir" aria-hidden="true">{{ cw(m) ? "↻" : "↺" }}</span>
        </button>
      </div>
      <div class="btn-row">
        <button type="button" class="btn btn--danger" :disabled="!motorTest.state.active" data-test="stop-motors" @click="motorTest.stop('requested')">
          {{ t("esc.motorTest.stop") }}
        </button>
      </div>
      <p class="hint" aria-live="polite">
        {{ motorTest.state.running ? t("esc.motorTest.running", { n: (motorTest.state.motor ?? 0) + 1 }) : t("esc.motorTest.idle") }}
      </p>
      <p v-if="showNoSpinHint" class="callout callout--warn" data-test="no-spin-hint">
        <strong>{{ t("esc.motorTest.noSpinTitle") }}</strong> {{ t("esc.motorTest.noSpinBody") }}
      </p>
      <p class="hint">{{ t("esc.motorTest.stopNote") }}</p>
      <ErrorPanel v-if="motorTest.state.error" :error="motorTest.state.error" />

      <p class="hint">{{ t("esc.motorTest.checkIntro") }}</p>
      <label class="choice"><input v-model="wizard.ui.motorOrderOk" type="checkbox" /> {{ t("esc.motorTest.orderOk") }}</label>
      <label class="choice"><input v-model="wizard.ui.motorDirectionOk" type="checkbox" /> {{ t("esc.motorTest.directionOk") }}</label>
    </template>

    <fieldset class="disabled-stub" aria-disabled="true">
      <legend>{{ t("esc.advancedChecks.title") }} <span class="badge">{{ t("common.phase2") }}</span></legend>
      <p class="hint">{{ t("esc.advancedChecks.note") }}</p>
      <label class="choice"><input type="checkbox" disabled /> {{ t("esc.advancedChecks.bidir") }}</label>
      <label class="choice"><input type="checkbox" disabled /> {{ t("esc.advancedChecks.esc") }}</label>
    </fieldset>
  </div>
</template>
