<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U4: receiver protocol, UART (port picker inside the step), telemetry question.
import { computed, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import PortPicker from "../../components/PortPicker.vue";
import ReflashOffer from "../../components/ReflashOffer.vue";
import { missingOptions, reflashBlock } from "../reflash";
import { optionList } from "../optionNames";
import type { RxProtocol } from "../../types/model";
import { asksTelemetry, rxProtocolLabel, SERIAL_RX_PROTOCOLS } from "../receiver";
import { markDirty, setReceiverProtocol, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t, te } = useI18n();
const rx = computed(() => wizard.model.receiver);

/**
 * Is this receiver in the board's build? Asks the backend which options it needs
 * (flasher_required_options; the mapping is backend-only) and compares with the board's
 * build options. Unknown (empty list, no backend answer) never blocks.
 */
async function checkOptions(): Promise<void> {
  const { kind, protocol } = wizard.model.receiver;
  if (kind === "NONE" || !protocol) {
    wizard.ui.rxMissing = null;
    return;
  }
  const missing = await missingOptions(wizard.fc, { receiver: { kind, protocol } });
  // Answer for an older choice: drop it.
  if (wizard.model.receiver.kind !== kind || wizard.model.receiver.protocol !== protocol) return;
  wizard.ui.rxMissing = missing ? { kind, protocol, missing } : null;
}
const missing = computed(() => {
  const m = wizard.ui.rxMissing;
  return m && m.kind === rx.value.kind && m.protocol === rx.value.protocol && m.missing.length ? m.missing : null;
});
/** Why "Update the firmware to add this" can't be offered (null = it can). */
const block = computed(() => reflashBlock(wizard));
const rxReq = computed(() => ({ receiver: { kind: rx.value.kind, protocol: rx.value.protocol } }));
watch(() => [rx.value.kind, rx.value.protocol, wizard.fc], () => void checkOptions());
onMounted(checkOptions);

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
          <span>{{ rxProtocolLabel(p, t, te) }}</span>
        </label>
      </fieldset>
      <PortPicker role="receiver" />
    </template>

    <div v-if="missing" class="callout callout--warn" role="alert" data-test="rx-option-missing">
      <p>{{ t("receiver.optionMissing", { protocol: rxProtocolLabel(rx.protocol, t, te), options: optionList(missing, t, te) }) }}</p>
      <template v-if="!block">
        <p>{{ t("receiver.optionMissingChoices") }}</p>
        <ReflashOffer return-to="receiver" :requirements="rxReq" />
      </template>
      <!-- No update possible: another protocol is the only way on; say so plainly. -->
      <p v-else data-test="rx-option-no-update">
        <strong>{{ t(block === "applied" ? "receiver.optionMissingApplied" : "receiver.optionMissingNoUpdate", { protocol: rxProtocolLabel(rx.protocol, t, te) }) }}</strong>
      </p>
    </div>

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
