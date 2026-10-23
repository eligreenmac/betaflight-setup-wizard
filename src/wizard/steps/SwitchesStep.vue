<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U5: ARM + a few modes on AUX switches, with live switch detection from device://rc-channels
// (mocked until the device crate lands; disabled with guidance when unavailable).
// PREARM / turtle / AIRMODE have their own explained sections; "Detect" assigns a switch
// by flipping it on the radio.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api, EVENTS } from "../../api/commands";
import { onEvent, type Unlisten } from "../../api/events";
import { isNotImplemented } from "../../errors/codes";
import { addModeSwitch, markDirty, removeModeSwitch, setAirmode, setModeRemoval, setPrearm, wizard } from "../state";
import { catalogState, loadCatalog } from "../../api/catalog";
import type { Airmode } from "../modelExt";
import {
  auxToChannelIndex,
  detectSwitch,
  inRange,
  MAX_AUX,
  MODE_KEYS,
  displayBoxName,
  GPS_MODES,
  modeAvailable,
  modeKeyOf,
  positionOf,
  SPECIAL_MODES,
  SWITCH_POSITIONS,
  usesNewModeNames,
  type ModeKey,
  type SwitchPosition,
} from "../switches";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const fw = computed(() => wizard.fc?.firmwareVersion ?? wizard.model.fc.firmwareVersion);
const channels = ref<number[] | null>(null);
const live = ref<"starting" | "on" | "unavailable" | "stopped">("starting");
let offs: Unlisten[] = [];

const used = computed(() => new Set(wizard.model.switches.map((s) => modeKeyOf(s.mode))));
const addable = computed(() =>
  MODE_KEYS.filter(
    (k) =>
      k !== "arm" &&
      k !== "turtle" &&
      !SPECIAL_MODES.includes(k) &&
      !used.value.has(k) &&
      available(k) &&
      (!GPS_MODES.includes(k) || wizard.model.gps.enabled),
  ),
);
/** Mode availability: the firmware catalog's mode list when loaded, else the built-in versions. */
const available = (k: ModeKey) => modeAvailable(k, fw.value, catalogState.catalog?.modes);
const removing = (m: "PREARM" | "FLIP OVER AFTER CRASH" | "AIR MODE") => wizard.ext.removeModes.includes(m);
const dshot = computed(() => wizard.model.esc.protocol.startsWith("DSHOT"));
const airmodeChoices: { value: Airmode | null; key: string }[] = [
  { value: "ALWAYS", key: "always" },
  { value: "SWITCH", key: "switch" },
  { value: "OFF", key: "off" },
  { value: null, key: "keep" },
];
const prearmChoice = computed(() => (used.value.has("prearm") ? "add" : removing("PREARM") ? "remove" : wizard.ui.prearmChoice === "skip" ? "skip" : null));

// ---- "Detect": remember the sticks/switches now, then pick the AUX channel that moves.
const detecting = ref<number | null>(null);
let baseline: number[] | null = null;
function startDetect(i: number): void {
  if (detecting.value === i) return void (detecting.value = null);
  detecting.value = i;
  baseline = channels.value ? [...channels.value] : null;
}
watch(channels, (now) => {
  if (detecting.value === null || !now) return;
  if (!baseline) return void (baseline = [...now]);
  const hit = detectSwitch(baseline, now);
  if (!hit) return;
  const i = detecting.value;
  detecting.value = null;
  baseline = null;
  setAux(i, hit.auxChannel);
  setPos(i, hit.position);
});
const positions = Object.keys(SWITCH_POSITIONS) as SwitchPosition[];

/** Mode label: translated name (the firmware's box name is shown as a hint when it differs by version). */
function label(mode: string): string {
  const k = modeKeyOf(mode);
  return k ? t(`switches.modes.${k}`) : mode;
}
const renamed = (k: ModeKey | null) => k === "altHold" || k === "posHold";

function add(k: ModeKey): void {
  addModeSwitch(wizard, k);
}
function remove(i: number): void {
  if (detecting.value !== null) detecting.value = null;
  removeModeSwitch(wizard, i);
}
function setAux(i: number, aux: number): void {
  wizard.model.switches[i]!.auxChannel = aux;
  markDirty();
}
function setPos(i: number, p: SwitchPosition): void {
  const [a, b] = SWITCH_POSITIONS[p];
  wizard.model.switches[i]!.rangeMin = a;
  wizard.model.switches[i]!.rangeMax = b;
  markDirty();
}
function active(sw: { auxChannel: number; rangeMin: number; rangeMax: number }): boolean | null {
  const v = channels.value?.[auxToChannelIndex(sw.auxChannel)];
  return v === undefined ? null : inRange(v, sw.rangeMin, sw.rangeMax);
}

onMounted(async () => {
  void loadCatalog(fw.value);
  if (!wizard.connectedPath) return void (live.value = "unavailable");
  offs.push(await onEvent(EVENTS.rcChannels, (p) => ((channels.value = p.channels), (live.value = "on"))));
  offs.push(await onEvent(EVENTS.rcStopped, () => (live.value = "stopped")));
  try {
    await api.device.rcStreamStart(15);
  } catch (e) {
    live.value = isNotImplemented(e) ? "unavailable" : "stopped";
  }
});
onBeforeUnmount(() => {
  offs.forEach((o) => o());
  offs = [];
  if (live.value !== "unavailable") void api.device.rcStreamStop().catch(() => undefined);
});
</script>

<template>
  <div>
    <h2>{{ t("steps.switches.title") }}</h2>
    <p class="lead">{{ t("steps.switches.description") }}</p>
    <p class="hint">{{ t(usesNewModeNames(fw) ? "switches.namesNew" : "switches.namesLegacy", { version: fw || "?" }) }}</p>

    <div v-if="live === 'unavailable'" class="callout callout--warn" data-test="rc-unavailable">{{ t("switches.liveUnavailable") }}</div>
    <div v-else-if="live === 'stopped'" class="callout callout--warn">{{ t("switches.liveStopped") }}</div>
    <p v-else class="hint">{{ t("switches.liveHint") }}</p>

    <div v-for="(sw, i) in wizard.model.switches" :key="i" class="switch-row" :data-mode="sw.mode">
      <div>
        <strong>{{ label(sw.mode) }}</strong>
        <small v-if="renamed(modeKeyOf(sw.mode))" class="hint" style="display: block">
          {{ t("switches.boxName", { name: displayBoxName(modeKeyOf(sw.mode)!, fw), version: fw || "?" }) }}
        </small>
      </div>
      <select :value="sw.auxChannel" :aria-label="t('switches.channel')" @change="setAux(i, Number(($event.target as HTMLSelectElement).value))">
        <option v-for="a in MAX_AUX" :key="a" :value="a - 1">AUX{{ a }}</option>
      </select>
      <select :value="positionOf(sw.rangeMin, sw.rangeMax) ?? ''" :aria-label="t('switches.position')" @change="setPos(i, ($event.target as HTMLSelectElement).value as SwitchPosition)">
        <option v-if="!positionOf(sw.rangeMin, sw.rangeMax)" value="">{{ t("switches.custom", { min: sw.rangeMin, max: sw.rangeMax }) }}</option>
        <option v-for="p in positions" :key="p" :value="p">{{ t(`switches.positions.${p}`) }}</option>
      </select>
      <span>
        <span v-if="active(sw) !== null" :class="['live', active(sw) ? 'live--on' : '']">{{ active(sw) ? t("switches.on") : t("switches.off") }}</span>
        <button
          v-if="live === 'on'"
          type="button"
          class="btn btn--small btn--link"
          :data-detect="i"
          :aria-pressed="detecting === i"
          @click="startDetect(i)"
        >
          {{ detecting === i ? t("switches.detecting") : t("switches.detect") }}
        </button>
        <button v-if="modeKeyOf(sw.mode) !== 'arm'" type="button" class="btn btn--small btn--link" @click="remove(i)">{{ t("switches.remove") }}</button>
      </span>
    </div>
    <p v-if="detecting !== null" class="hint" role="status" data-test="detect-hint">{{ t("switches.detectHint") }}</p>

    <div v-if="addable.length" class="btn-row">
      <button v-for="k in addable" :key="k" type="button" class="btn btn--small" :data-add="k" @click="add(k)">+ {{ t(`switches.modes.${k}`) }}</button>
    </div>
    <p class="hint">{{ t("switches.armHint") }}</p>

    <section v-if="available('prearm')" class="subcard" data-section="prearm">
      <h3>{{ t("switches.prearm.title") }}</h3>
      <p>{{ t("switches.prearm.body") }}</p>
      <div class="choice-col" role="radiogroup" :aria-label="t('switches.prearm.title')">
        <label v-for="c in (['add', 'skip', 'remove'] as const)" :key="c">
          <input type="radio" name="prearm" :data-prearm="c" :checked="prearmChoice === c" @change="setPrearm(wizard, c)" />
          <span>
            <strong>{{ t(`switches.prearm.${c}`) }}</strong>
            <small class="hint" style="display: block">{{ t(`switches.prearm.${c}Hint`) }}</small>
          </span>
        </label>
      </div>
    </section>

    <section v-if="available('turtle')" class="subcard" data-section="turtle">
      <h3>{{ t("switches.turtle.title") }}</h3>
      <p>{{ t("switches.turtle.body") }}</p>
      <p :class="['hint', dshot ? '' : 'hint--warn']">{{ dshot ? t("switches.turtle.dshotOk") : t("switches.turtle.needsDshot") }}</p>
      <button v-if="!used.has('turtle')" type="button" class="btn btn--small" data-add="turtle" @click="add('turtle')">+ {{ t("switches.modes.turtle") }}</button>
      <p v-else class="hint">{{ t("switches.turtle.added") }}</p>
      <label v-if="!used.has('turtle')" class="choice">
        <input
          type="checkbox"
          data-remove-mode="turtle"
          :checked="removing('FLIP OVER AFTER CRASH')"
          @change="setModeRemoval(wizard, 'FLIP OVER AFTER CRASH', ($event.target as HTMLInputElement).checked)"
        />
        {{ t("switches.removeExisting", { mode: t("switches.modes.turtle") }) }}
      </label>
    </section>

    <section v-if="available('airMode')" class="subcard" data-section="airmode">
      <h3>{{ t("switches.airmode.title") }}</h3>
      <p>{{ t("switches.airmode.body") }}</p>
      <div class="choice-col" role="radiogroup" :aria-label="t('switches.airmode.title')">
        <label v-for="c in airmodeChoices" :key="c.key">
          <input type="radio" name="airmode" :data-airmode="c.key" :checked="wizard.ext.airmode === c.value" @change="setAirmode(wizard, c.value)" />
          <span>
            <strong>{{ t(`switches.airmode.${c.key}`) }}</strong>
            <small class="hint" style="display: block">{{ t(`switches.airmode.${c.key}Hint`) }}</small>
          </span>
        </label>
      </div>
      <label v-if="wizard.ext.airmode !== 'SWITCH'" class="choice">
        <input
          type="checkbox"
          data-remove-mode="airMode"
          :checked="removing('AIR MODE')"
          @change="setModeRemoval(wizard, 'AIR MODE', ($event.target as HTMLInputElement).checked)"
        />
        {{ t("switches.removeExisting", { mode: t("switches.modes.airMode") }) }}
      </label>
      <p class="hint">{{ t("switches.removeHint") }}</p>
    </section>
  </div>
</template>
