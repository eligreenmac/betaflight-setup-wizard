<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// Which beeps the quad makes. "Keep" = beepers stay as they are on the board (null);
// "Choose" starts from the recommendation (Betaflight defaults, but no beeping on USB
// power) and stores the firmware names to turn off. Only the beepers the detected
// firmware has are shown; the forced demo shows the full list.
import { computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { catalogState, loadCatalog } from "../../api/catalog";
import {
  availableBeaconEvents,
  availableBeepers,
  BEEPER_GROUPS,
  isBeeperOn,
  KEEP_ON,
  recommendedDisabled,
  setBeacon,
  setBeeper,
  type BeeperDef,
} from "../beepers";
import type { DshotBeaconCondition } from "../../types/pendingModel";
import { markDirty, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const fwVersion = computed(() => wizard.fc?.firmwareVersion || wizard.model.fc.firmwareVersion || null);
onMounted(() => void loadCatalog(fwVersion.value));
/** The firmware's lists (config_firmware_catalog); null until loaded = the full #65 list. */
const catBeepers = computed(() => catalogState.catalog?.beepers ?? null);
const list = computed(() => availableBeepers(catBeepers.value));
const groups = computed(() => BEEPER_GROUPS.map((g) => ({ g, items: list.value.filter((b) => b.group === g) })).filter((x) => x.items.length));
const disabled = computed(() => wizard.ext.beepers?.disabled ?? []);
const isRecommended = computed(() => {
  const rec = new Set(recommendedDisabled(catBeepers.value));
  return disabled.value.length === rec.size && disabled.value.every((n) => rec.has(n));
});

function setMode(mode: "keep" | "choose"): void {
  wizard.ui.beeperMode = mode;
  if (mode === "keep") wizard.ext.beepers = null;
  else wizard.ext.beepers ??= { disabled: recommendedDisabled(catBeepers.value) };
  markDirty();
}
function toggle(b: BeeperDef, on: boolean): void {
  if (!wizard.ext.beepers) return;
  wizard.ext.beepers = { ...wizard.ext.beepers, disabled: setBeeper(wizard.ext.beepers.disabled, b, on, catBeepers.value) };
  markDirty();
}
function reset(): void {
  wizard.ext.beepers = { ...wizard.ext.beepers, disabled: recommendedDisabled(catBeepers.value) };
  markDirty();
}
const locked = (b: BeeperDef) => KEEP_ON.includes(b.name);

// ---- DShot beacon (the motors beep): optional, null = keep.
const dshot = computed(() => wizard.model.esc.protocol.startsWith("DSHOT"));
const beaconEvents = computed(() => availableBeaconEvents(catalogState.catalog?.beaconEvents ?? null));
const beacon = computed(() => wizard.ext.beepers?.dshotBeaconDisabled ?? null);
function setBeaconMode(change: boolean): void {
  if (!wizard.ext.beepers) return;
  // Changing starts from Betaflight's default: the beacon is off for both events.
  wizard.ext.beepers = { ...wizard.ext.beepers, dshotBeaconDisabled: change ? (beacon.value ?? [...beaconEvents.value]) : null };
  markDirty();
}
function toggleBeacon(e: DshotBeaconCondition, on: boolean): void {
  if (!wizard.ext.beepers || !beacon.value) return;
  wizard.ext.beepers = { ...wizard.ext.beepers, dshotBeaconDisabled: setBeacon(beacon.value, e, on) };
  markDirty();
}
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
          <li v-for="b in grp.items" :key="b.name">
            <label>
              <input
                type="checkbox"
                :data-beeper="b.name"
                :checked="isBeeperOn(disabled, b)"
                :disabled="locked(b)"
                @change="toggle(b, ($event.target as HTMLInputElement).checked)"
              />
              <span>
                <strong>{{ t(`beepers.items.${b.name}`) }}</strong>
                <small class="mono" dir="ltr"> {{ b.name }}</small>
                <small class="hint" style="display: block">{{ t(`beepers.explain.${b.name}`) }}</small>
                <small v-if="locked(b)" class="hint" style="display: block">{{ t("beepers.locked") }}</small>
              </span>
            </label>
          </li>
        </ul>
      </section>

      <section v-if="dshot && beaconEvents.length" class="subcard" data-section="beacon">
        <h3>{{ t("beepers.beacon.title") }}</h3>
        <p>{{ t("beepers.beacon.body") }}</p>
        <div class="choice-row" role="radiogroup" :aria-label="t('beepers.beacon.title')">
          <label><input type="radio" name="beacon" data-beacon="keep" :checked="beacon === null" @change="setBeaconMode(false)" /> {{ t("beepers.beacon.keep") }}</label>
          <label><input type="radio" name="beacon" data-beacon="change" :checked="beacon !== null" @change="setBeaconMode(true)" /> {{ t("beepers.beacon.change") }}</label>
        </div>
        <ul v-if="beacon !== null">
          <li v-for="e in beaconEvents" :key="e">
            <label>
              <input type="checkbox" :data-beacon-event="e" :checked="!beacon.includes(e)" @change="toggleBeacon(e, ($event.target as HTMLInputElement).checked)" />
              {{ t(`beepers.beacon.events.${e}`) }}
            </label>
          </li>
        </ul>
      </section>
    </template>
  </div>
</template>
