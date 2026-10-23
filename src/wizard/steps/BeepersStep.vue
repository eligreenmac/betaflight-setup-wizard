<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// Which beeps the quad makes. "Keep" = beepers stay as they are on the board (null);
// "Choose" starts from the recommendation (Betaflight defaults, but no beeping on USB
// power) and stores the firmware names to turn off. Only the beepers the detected
// firmware has are shown; the forced demo shows the full list.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { demoState } from "../../api/fallback";
import { availableBeepers, BEEPER_GROUPS, beeperName, isBeeperOn, KEEP_ON, recommendedDisabled, setBeeper, type BeeperDef } from "../beepers";
import { markDirty, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const fw = computed<string | null>(() => (demoState.forced ? null : wizard.fc?.firmwareVersion || wizard.model.fc.firmwareVersion || null));
const list = computed(() => availableBeepers(fw.value));
const groups = computed(() => BEEPER_GROUPS.map((g) => ({ g, items: list.value.filter((b) => b.group === g) })).filter((x) => x.items.length));
const disabled = computed(() => wizard.ext.beepers?.disabled ?? []);
const isRecommended = computed(() => {
  const rec = new Set(recommendedDisabled(fw.value));
  return disabled.value.length === rec.size && disabled.value.every((n) => rec.has(n));
});

function setMode(mode: "keep" | "choose"): void {
  wizard.ui.beeperMode = mode;
  if (mode === "keep") wizard.ext.beepers = null;
  else wizard.ext.beepers ??= { disabled: recommendedDisabled(fw.value) };
  markDirty();
}
function toggle(b: BeeperDef, on: boolean): void {
  if (!wizard.ext.beepers) return;
  wizard.ext.beepers = { disabled: setBeeper(wizard.ext.beepers.disabled, b, on, fw.value) };
  markDirty();
}
function reset(): void {
  wizard.ext.beepers = { disabled: recommendedDisabled(fw.value) };
  markDirty();
}
const locked = (b: BeeperDef) => KEEP_ON.includes(b.key);
</script>

<template>
  <div>
    <h2>{{ t("steps.beepers.title") }}</h2>
    <p class="lead">{{ t("steps.beepers.description") }}</p>

    <div class="choice-col" role="radiogroup" :aria-label="t('steps.beepers.title')">
      <label>
        <input type="radio" name="beeper-mode" data-beepers="choose" :checked="wizard.ui.beeperMode === 'choose'" @change="setMode('choose')" />
        <span>
          <strong>{{ t("beepers.choose") }}</strong>
          <small class="hint" style="display: block">{{ t("beepers.chooseHint") }}</small>
        </span>
      </label>
      <label>
        <input type="radio" name="beeper-mode" data-beepers="keep" :checked="wizard.ui.beeperMode === 'keep'" @change="setMode('keep')" />
        <span>
          <strong>{{ t("beepers.keep") }}</strong>
          <small class="hint" style="display: block">{{ t("beepers.keepHint") }}</small>
        </span>
      </label>
    </div>

    <template v-if="wizard.ext.beepers">
      <div class="btn-row">
        <button type="button" class="btn btn--small" data-test="beepers-reset" :disabled="isRecommended" @click="reset">{{ t("beepers.reset") }}</button>
        <span v-if="isRecommended" class="hint" data-test="beepers-recommended">{{ t("beepers.isRecommended") }}</span>
      </div>
      <p class="callout" data-test="keep-on-note">{{ t("beepers.keepOnNote") }}</p>
      <section v-for="grp in groups" :key="grp.g" class="beeper-group" :data-group="grp.g">
        <h3>{{ t(`beepers.groups.${grp.g}`) }}</h3>
        <p class="hint">{{ t(`beepers.groupHints.${grp.g}`) }}</p>
        <ul>
          <li v-for="b in grp.items" :key="b.key">
            <label>
              <input
                type="checkbox"
                :data-beeper="b.key"
                :checked="isBeeperOn(disabled, b, fw)"
                :disabled="locked(b)"
                @change="toggle(b, ($event.target as HTMLInputElement).checked)"
              />
              <span>
                <strong>{{ t(`beepers.items.${b.key}`) }}</strong>
                <small class="mono" dir="ltr"> {{ beeperName(b, fw) }}</small>
                <small class="hint" style="display: block">{{ t(`beepers.explain.${b.key}`) }}</small>
                <small v-if="locked(b)" class="hint" style="display: block">{{ t("beepers.locked") }}</small>
              </span>
            </label>
          </li>
        </ul>
      </section>
    </template>
  </div>
</template>
