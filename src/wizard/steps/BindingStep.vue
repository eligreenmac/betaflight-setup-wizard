<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// ExpressLRS binding (ARCHITECTURE §5.1 "ELRS binding phrase"). Onboard SPI ExpressLRS:
// the phrase is hashed with api.config.elrsUid and only the 6-byte UID goes into the model
// (written as expresslrs_uid). A UART receiver keeps its own phrase, so here we only explain.
// The phrase lives in this component only: never in the model, storage or logs, and it
// is cleared as soon as it is hashed and when the step is left.
import { computed, onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api } from "../../api/commands";
import ErrorPanel from "../../components/ErrorPanel.vue";
import { toUiError, type UiError } from "../../errors/codes";
import { isElrs } from "../receiver";
import { markDirty, wizard, type CrsfFamily, type ElrsBinding } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const rx = computed(() => wizard.model.receiver);
const onboard = computed(() => rx.value.kind === "SPI");
const elrs = computed(() => isElrs(wizard));
const uidSet = computed(() => rx.value.elrsUid?.length === 6);

const phrase = ref("");
const show = ref(false);
const busy = ref(false);
const error = ref<UiError | null>(null);
const editing = ref(!uidSet.value);
const edgeSpaces = computed(() => phrase.value !== phrase.value.trim() && phrase.value.trim() !== "");

function setFamily(f: CrsfFamily): void {
  wizard.ui.crsfFamily = f;
  wizard.ui.elrsBinding = null;
  markDirty();
}
function setBinding(b: ElrsBinding): void {
  wizard.ui.elrsBinding = b;
  if (b === "button") wizard.model.receiver.elrsUid = null; // keep the board's current UID
  error.value = null;
  markDirty();
}

async function usePhrase(): Promise<void> {
  if (!phrase.value.trim() || busy.value) return;
  busy.value = true;
  error.value = null;
  try {
    const uid = await api.config.elrsUid(phrase.value);
    wizard.model.receiver.elrsUid = uid;
    markDirty();
    editing.value = false;
  } catch (e) {
    error.value = toUiError(e);
  } finally {
    phrase.value = ""; // drop it right away, success or not
    busy.value = false;
  }
}

function change(): void {
  editing.value = true;
}

onBeforeUnmount(() => {
  phrase.value = "";
});
</script>

<template>
  <div>
    <h2>{{ t("steps.binding.title") }}</h2>
    <p class="lead">{{ t("steps.binding.description") }}</p>

    <fieldset v-if="!onboard">
      <legend>{{ t("binding.familyQuestion") }}</legend>
      <label class="choice">
        <input type="radio" name="crsf-family" :checked="wizard.ui.crsfFamily === 'ELRS'" data-family="ELRS" @change="setFamily('ELRS')" />
        <span>{{ t("binding.familyElrs") }}</span>
      </label>
      <label class="choice">
        <input type="radio" name="crsf-family" :checked="wizard.ui.crsfFamily === 'CROSSFIRE'" data-family="CROSSFIRE" @change="setFamily('CROSSFIRE')" />
        <span>{{ t("binding.familyCrossfire") }}</span>
      </label>
      <p v-if="wizard.ui.crsfFamily === 'CROSSFIRE'" class="callout callout--info" data-test="crossfire-note">{{ t("binding.crossfireNote") }}</p>
    </fieldset>

    <template v-if="elrs">
      <div class="callout callout--info">
        <h3>{{ t("binding.whatTitle") }}</h3>
        <p>{{ t("binding.what") }}</p>
        <p><strong>{{ t("binding.mustMatch") }}</strong></p>
        <p>{{ t("binding.whereToFind") }}</p>
      </div>

      <fieldset>
        <legend>{{ t("binding.howQuestion") }}</legend>
        <label class="choice">
          <input type="radio" name="elrs-binding" :checked="wizard.ui.elrsBinding === 'phrase'" data-binding="phrase" @change="setBinding('phrase')" />
          <span>{{ t("binding.usePhrase") }}</span>
        </label>
        <label class="choice">
          <input type="radio" name="elrs-binding" :checked="wizard.ui.elrsBinding === 'button'" data-binding="button" @change="setBinding('button')" />
          <span>{{ t("binding.useButton") }}</span>
        </label>
      </fieldset>

      <template v-if="wizard.ui.elrsBinding === 'phrase' && onboard">
        <div v-if="uidSet && !editing" class="callout callout--ok" data-test="phrase-set">
          <p>{{ t("binding.phraseSet") }}</p>
          <div class="btn-row"><button type="button" class="btn btn--small" data-test="phrase-change" @click="change">{{ t("binding.change") }}</button></div>
        </div>
        <div v-else class="field">
          <label for="elrs-phrase">{{ t("binding.phraseLabel") }}</label>
          <input
            id="elrs-phrase"
            v-model="phrase"
            :type="show ? 'text' : 'password'"
            autocomplete="off"
            autocapitalize="off"
            spellcheck="false"
            dir="auto"
            data-test="phrase"
            @keydown.enter.prevent="usePhrase"
          />
          <label class="choice"><input v-model="show" type="checkbox" data-test="phrase-show" /> {{ t("binding.showPhrase") }}</label>
          <p v-if="edgeSpaces" class="hint" data-test="phrase-spaces">{{ t("binding.spacesWarning") }}</p>
          <p class="hint">{{ t("binding.notSaved") }}</p>
          <div class="btn-row">
            <button type="button" class="btn btn--primary" :disabled="!phrase.trim() || busy" data-test="phrase-apply" @click="usePhrase">{{ t("binding.apply") }}</button>
          </div>
        </div>
      </template>

      <div v-if="wizard.ui.elrsBinding === 'phrase' && !onboard" class="callout callout--info" data-test="uart-phrase">
        <p>{{ t("binding.uartPhrase") }}</p>
        <ol>
          <li>{{ t("binding.uartPhrase1") }}</li>
          <li>{{ t("binding.uartPhrase2") }}</li>
        </ol>
      </div>

      <div v-if="wizard.ui.elrsBinding === 'button'" class="callout callout--info" data-test="bind-steps">
        <h3>{{ t("binding.buttonTitle") }}</h3>
        <ol>
          <li>{{ t(onboard ? "binding.buttonSpi1" : "binding.buttonUart1") }}</li>
          <li>{{ t("binding.button2") }}</li>
          <li>{{ t("binding.button3") }}</li>
        </ol>
        <p class="hint">{{ t(onboard ? "binding.buttonSpiKeep" : "binding.buttonLater") }}</p>
      </div>

      <ErrorPanel v-if="error" :error="error" :busy="busy" @action="usePhrase" />
    </template>
  </div>
</template>
