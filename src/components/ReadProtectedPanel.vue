<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// READ_PROTECTED from flasher_flash: explain, then remove the protection ONLY after an
// explicit typed confirmation, because it erases the whole chip (ARCHITECTURE §2.3).
// Nothing is erased before the user confirms; the parent continues into flashing.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { toUiError, type UiError } from "../errors/codes";
import { removeReadProtection } from "../wizard/flash";
import ErrorPanel from "./ErrorPanel.vue";

const props = defineProps<{ dfuSerial: string | null; hasBackup: boolean }>();
const emit = defineEmits<{ unprotected: [] }>();
const { t } = useI18n();

const understood = ref(false);
const typed = ref("");
const busy = ref(false);
const error = ref<UiError | null>(null);

const word = computed(() => t("readProtect.confirmWord"));
const norm = (s: string) => s.trim().toLocaleUpperCase();
const confirmed = computed(() => understood.value && norm(typed.value) === norm(word.value));

async function erase(): Promise<void> {
  if (!confirmed.value || busy.value) return;
  busy.value = true;
  error.value = null;
  try {
    await removeReadProtection(props.dfuSerial);
    emit("unprotected");
  } catch (e) {
    error.value = toUiError(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <section class="callout callout--danger" role="alert" data-test="read-protected">
    <h3>{{ t("readProtect.title") }}</h3>
    <p>{{ t("readProtect.what") }}</p>
    <p>{{ t("readProtect.why") }}</p>
    <p><strong>{{ t("readProtect.eraseWarning") }}</strong></p>
    <ul>
      <li>{{ t("readProtect.lose1") }}</li>
      <li>{{ t("readProtect.lose2") }}</li>
      <li>{{ t(hasBackup ? "readProtect.backupSaved" : "readProtect.noBackup") }}</li>
    </ul>
    <p>{{ t("readProtect.safe") }}</p>

    <label class="choice">
      <input v-model="understood" type="checkbox" :disabled="busy" data-test="rp-understood" />
      {{ t("readProtect.understand") }}
    </label>
    <div class="field">
      <label for="rp-confirm">{{ t("readProtect.typeToConfirm", { word }) }}</label>
      <input id="rp-confirm" v-model="typed" type="text" autocomplete="off" spellcheck="false" :disabled="busy" data-test="rp-confirm" />
    </div>
    <div class="btn-row">
      <button type="button" class="btn btn--danger" :disabled="!confirmed || busy" data-test="rp-erase" @click="erase">
        {{ busy ? t("readProtect.erasing") : t("readProtect.erase") }}
      </button>
    </div>
    <p class="hint">{{ t("readProtect.leave") }}</p>
    <!-- Retrying is the erase button above (still confirmed). -->
    <ErrorPanel v-if="error" :error="error" :busy="busy" :hide="['retryFlash', 'retry', 'installDriver']" />
  </section>
</template>
