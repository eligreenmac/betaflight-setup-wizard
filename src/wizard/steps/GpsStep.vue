<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U4: GPS protocol and UART.
import { useI18n } from "vue-i18n";
import PortPicker from "../../components/PortPicker.vue";
import type { GpsProvider } from "../../types/model";
import { markDirty, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();
const providers: GpsProvider[] = ["UBLOX", "NMEA"];
function setProvider(p: GpsProvider): void {
  wizard.model.gps.provider = p;
  markDirty();
}
</script>

<template>
  <div>
    <h2>{{ t("steps.gps.title") }}</h2>
    <p class="lead">{{ t("steps.gps.description") }}</p>
    <fieldset>
      <legend>{{ t("gps.provider") }}</legend>
      <label v-for="p in providers" :key="p" class="choice">
        <input type="radio" name="gps-provider" :checked="wizard.model.gps.provider === p" @change="setProvider(p)" />
        <span>{{ t(`gps.providers.${p}`) }}</span>
      </label>
    </fieldset>
    <PortPicker role="gps" />
    <p class="hint">{{ t("gps.hint") }}</p>
  </div>
</template>
