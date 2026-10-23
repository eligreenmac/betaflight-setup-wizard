<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U4: VTX control protocol / digital system and its UART.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import PortPicker from "../../components/PortPicker.vue";
import type { DigitalSystem, VtxProtocol } from "../../types/model";
import { markDirty, setVtxProtocol, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();
const vtx = computed(() => wizard.model.vtx);
const analogProtocols: VtxProtocol[] = ["SMARTAUDIO", "TRAMP"];
const systems: DigitalSystem[] = ["DJI", "WALKSNAIL", "HDZERO", "OTHER"];

function setSystem(s: DigitalSystem): void {
  wizard.model.vtx.digitalSystem = s;
  markDirty();
}
</script>

<template>
  <div>
    <h2>{{ t("steps.vtx.title") }}</h2>
    <p class="lead">{{ t("steps.vtx.description") }}</p>

    <fieldset v-if="vtx.kind === 'ANALOG'">
      <legend>{{ t("vtx.protocol") }}</legend>
      <label v-for="p in analogProtocols" :key="p" class="choice">
        <input type="radio" name="vtx-protocol" :checked="vtx.protocol === p" @change="setVtxProtocol(wizard, p)" />
        <span>{{ t(`vtx.protocols.${p}`) }}</span>
      </label>
    </fieldset>

    <fieldset v-else>
      <legend>{{ t("vtx.system") }}</legend>
      <label v-for="s in systems" :key="s" class="choice">
        <input type="radio" name="vtx-system" :checked="vtx.digitalSystem === s" @change="setSystem(s)" />
        <span>{{ t(`vtx.systems.${s}`) }}</span>
      </label>
      <p class="hint">{{ t("vtx.digitalHint") }}</p>
    </fieldset>

    <PortPicker role="vtx" />
  </div>
</template>
