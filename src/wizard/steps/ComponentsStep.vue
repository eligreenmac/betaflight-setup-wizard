<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U4: what is wired to the flight controller. Shows/hides the receiver/vtx/gps steps.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { ReceiverKind, VtxKind } from "../../types/model";
import { hasOnboardElrs } from "../receiver";
import { setEscTelemetry, setGpsEnabled, setReceiverKind, setVtxKind, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const onboard = computed(() => hasOnboardElrs(wizard.fc));
// Onboard SPI ExpressLRS is offered only when the board reports it. Soft serial, PPM and MSP RX are out of the MVP.
const rxKinds = computed<ReceiverKind[]>(() => (onboard.value ? ["SERIAL", "SPI", "NONE"] : ["SERIAL", "NONE"]));
const vtxKinds: VtxKind[] = ["DIGITAL", "ANALOG", "NONE"];
</script>

<template>
  <div>
    <h2>{{ t("steps.components.title") }}</h2>
    <p class="lead">{{ t("steps.components.description") }}</p>

    <fieldset>
      <legend>{{ t("components.receiver") }}</legend>
      <label v-for="k in rxKinds" :key="k" class="choice">
        <input type="radio" name="rx-kind" :value="k" :checked="wizard.model.receiver.kind === k" :data-rx-kind="k" @change="setReceiverKind(wizard, k)" />
        <span>{{ t(`components.rxKind.${k}`) }}<small>{{ t(`components.rxKindHint.${k}`) }}</small></span>
      </label>
    </fieldset>

    <fieldset>
      <legend>{{ t("components.vtx") }}</legend>
      <label v-for="k in vtxKinds" :key="k" class="choice">
        <input type="radio" name="vtx-kind" :value="k" :checked="wizard.model.vtx.kind === k" :data-vtx-kind="k" @change="setVtxKind(wizard, k)" />
        <span>{{ t(`components.vtxKind.${k}`) }}<small>{{ t(`components.vtxKindHint.${k}`) }}</small></span>
      </label>
    </fieldset>

    <fieldset>
      <legend>{{ t("components.other") }}</legend>
      <label class="choice">
        <input type="checkbox" :checked="wizard.model.gps.enabled" data-test="gps" @change="setGpsEnabled(wizard, ($event.target as HTMLInputElement).checked)" />
        <span>{{ t("components.gps") }}<small>{{ t("components.gpsHint") }}</small></span>
      </label>
      <label class="choice">
        <input type="checkbox" :checked="wizard.ui.escTelemetry" @change="setEscTelemetry(wizard, ($event.target as HTMLInputElement).checked)" />
        <span>{{ t("components.escTelemetry") }}<small>{{ t("components.escTelemetryHint") }}</small></span>
      </label>
    </fieldset>
    <p class="hint">{{ t("components.softSerialNote") }}</p>
  </div>
</template>
