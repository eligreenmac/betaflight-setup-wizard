<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U4: receiver protocol, UART (port picker inside the step), telemetry question.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import PortPicker from "../../components/PortPicker.vue";
import type { RxProtocol } from "../../types/model";
import { asksTelemetry, SERIAL_RX_PROTOCOLS } from "../receiver";
import { markDirty, setReceiverProtocol, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();
const rx = computed(() => wizard.model.receiver);

function setTelemetry(v: boolean): void {
  wizard.model.receiver.telemetry = v;
  markDirty();
}
function setFlag(k: "inverted" | "halfDuplex", v: boolean): void {
  wizard.model.receiver[k] = v;
  markDirty();
}
</script>

<template>
  <div>
    <h2>{{ t("steps.receiver.title") }}</h2>
    <p class="lead">{{ t("steps.receiver.description") }}</p>

    <template v-if="rx.kind === 'SPI'">
      <div class="callout callout--info">
        <h3>{{ t("receiver.onboardTitle") }}</h3>
        <p>{{ t("receiver.onboardBody") }}</p>
      </div>
    </template>

    <template v-else>
      <fieldset>
        <legend>{{ t("receiver.protocol") }}</legend>
        <label v-for="p in SERIAL_RX_PROTOCOLS" :key="p" class="choice">
          <input type="radio" name="rx-protocol" :checked="rx.protocol === p" :data-protocol="p" @change="setReceiverProtocol(wizard, p as RxProtocol)" />
          <span>{{ t(`receiver.protocols.${p}`) }}</span>
        </label>
      </fieldset>
      <PortPicker role="receiver" />
    </template>

    <fieldset v-if="asksTelemetry(rx.protocol)">
      <legend>{{ t("receiver.telemetryQuestion") }}</legend>
      <label class="choice"><input type="radio" name="rx-telem" :checked="rx.telemetry" @change="setTelemetry(true)" /> {{ t("receiver.telemetryYes") }}</label>
      <label class="choice"><input type="radio" name="rx-telem" :checked="!rx.telemetry" @change="setTelemetry(false)" /> {{ t("receiver.telemetryNo") }}</label>
      <p class="hint">{{ t("receiver.telemetryHint") }}</p>
    </fieldset>

    <p v-if="rx.protocol === 'CRSF' || rx.protocol === 'EXPRESSLRS'" class="hint">{{ t("receiver.elrsHint") }}</p>

    <details v-if="rx.kind === 'SERIAL'">
      <summary>{{ t("receiver.advanced") }}</summary>
      <label class="choice"><input type="checkbox" :checked="rx.inverted" @change="setFlag('inverted', ($event.target as HTMLInputElement).checked)" /> {{ t("receiver.inverted") }}</label>
      <label class="choice"><input type="checkbox" :checked="rx.halfDuplex" @change="setFlag('halfDuplex', ($event.target as HTMLInputElement).checked)" /> {{ t("receiver.halfDuplex") }}</label>
    </details>
  </div>
</template>
