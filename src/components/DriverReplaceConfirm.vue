<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// Shown when installing WinUSB would replace the DFU device's current driver
// (DriverStatus.devices[].replacesDriver, e.g. STTub30). The parent installs only after
// `confirm`; nothing is installed from here without the checkbox.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { UsbDriverInfo } from "../types/model";
import { replacedDriverNames } from "../wizard/dfuDriver";

const props = defineProps<{ replaces: UsbDriverInfo[]; busy?: boolean }>();
const emit = defineEmits<{ confirm: []; cancel: [] }>();
const { t } = useI18n();

const understood = ref(false);
const names = computed(() => replacedDriverNames(props.replaces));
const driver = computed(() => (names.value.length ? names.value.join(", ") : t("driverReplace.unknownDriver")));
const devices = computed(() => [...new Set(props.replaces.map((d) => d.name))].join(", "));

function confirm(): void {
  if (understood.value && !props.busy) emit("confirm");
}
</script>

<template>
  <section class="callout callout--warn" role="alert" data-test="driver-replace">
    <h3>{{ t("driverReplace.title") }}</h3>
    <p>{{ t("driverReplace.body", { device: devices, driver }) }}</p>
    <p><strong>{{ t("driverReplace.tools") }}</strong></p>
    <p>{{ t("driverReplace.undo") }}</p>
    <label class="choice">
      <input v-model="understood" type="checkbox" :disabled="busy" data-test="drv-understood" />
      {{ t("driverReplace.understand", { driver }) }}
    </label>
    <div class="btn-row">
      <button type="button" class="btn btn--primary" :disabled="!understood || busy" data-test="drv-install" @click="confirm">
        {{ t("driverReplace.install") }}
      </button>
      <button type="button" class="btn" :disabled="busy" data-test="drv-cancel" @click="emit('cancel')">{{ t("driverReplace.cancel") }}</button>
    </div>
  </section>
</template>
