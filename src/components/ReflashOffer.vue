<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// "Update the firmware to add this" (wizard/reflash.ts): a clear confirmation (the board
// restarts, its settings are backed up first, the answers are kept), then the Firmware step.
// Not offered after any config_apply in this session, or when flasher_flash would mix
// simulated and real data; the reason is shown instead.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { FirmwareRequirements } from "../types/model";
import { goTo } from "../wizard/nav";
import { reflashBlock, requestReflash } from "../wizard/reflash";
import { wizard } from "../wizard/state";

const props = defineProps<{ returnTo: string; requirements: FirmwareRequirements }>();
const { t } = useI18n();
const confirming = ref(false);
const block = computed(() => reflashBlock(wizard));

function go(): void {
  confirming.value = false;
  if (requestReflash(wizard, { returnTo: props.returnTo, requirements: props.requirements })) goTo("firmware");
}
</script>

<template>
  <div class="reflash-offer" data-test="reflash">
    <p v-if="block === 'applied'" class="hint" data-test="reflash-blocked">{{ t("reflash.blockedApplied") }}</p>
    <p v-else-if="block === 'mixed'" class="hint" data-test="reflash-blocked">{{ t("reflash.blockedMixed") }}</p>
    <template v-else-if="!block">
      <div v-if="confirming" class="callout callout--warn" role="alertdialog" aria-labelledby="reflash-confirm-title" data-test="reflash-confirm">
        <h4 id="reflash-confirm-title">{{ t("reflash.confirmTitle") }}</h4>
        <p>{{ t("reflash.confirmBody") }}</p>
        <p>{{ t("reflash.confirmKeep") }}</p>
        <div class="btn-row">
          <button type="button" class="btn btn--primary" data-test="reflash-go" @click="go">{{ t("reflash.confirmYes") }}</button>
          <button type="button" class="btn" data-test="reflash-cancel" @click="confirming = false">{{ t("reflash.cancel") }}</button>
        </div>
      </div>
      <button v-else type="button" class="btn btn--small btn--primary" data-test="reflash-offer" @click="confirming = true">
        {{ t("reflash.offer") }}
      </button>
    </template>
  </div>
</template>
