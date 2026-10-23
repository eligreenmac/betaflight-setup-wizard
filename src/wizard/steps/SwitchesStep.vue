<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U5: ARM + a few modes on AUX switches, with live switch detection from rc://channels
// (mocked until the device crate lands; disabled with guidance when unavailable).
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { onEvent, type Unlisten } from "../../api/events";
import { UPCOMING_EVENTS, upcoming } from "../../api/upcoming";
import { isNotImplemented } from "../../errors/codes";
import { markDirty, wizard } from "../state";
import {
  auxToChannelIndex,
  inRange,
  MAX_AUX,
  MODE_KEYS,
  modeKeyOf,
  modeName,
  positionOf,
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
const addable = computed(() => MODE_KEYS.filter((k) => k !== "arm" && !used.value.has(k) && (k !== "posHold" || wizard.model.gps.enabled)));
const positions = Object.keys(SWITCH_POSITIONS) as SwitchPosition[];

/** Mode label: translated name + the firmware's box name when it differs by version. */
function label(mode: string): string {
  const k = modeKeyOf(mode);
  return k ? t(`switches.modes.${k}`) : mode;
}
const renamed = (k: ModeKey | null) => k === "altHold" || k === "posHold";

function add(k: ModeKey): void {
  const auxUsed = new Set(wizard.model.switches.map((s) => s.auxChannel));
  let aux = 0;
  while (auxUsed.has(aux) && aux < MAX_AUX - 1) aux++;
  wizard.model.switches.push({ mode: modeName(k, fw.value), auxChannel: aux, rangeMin: SWITCH_POSITIONS.high[0], rangeMax: SWITCH_POSITIONS.high[1] });
  markDirty();
}
function remove(i: number): void {
  wizard.model.switches.splice(i, 1);
  markDirty();
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
  if (!wizard.connectedPath) return void (live.value = "unavailable");
  offs.push(await onEvent(UPCOMING_EVENTS.rcChannels, (p) => ((channels.value = p.channels), (live.value = "on"))));
  offs.push(await onEvent(UPCOMING_EVENTS.rcStopped, () => (live.value = "stopped")));
  try {
    await upcoming.rcChannelsStreamStart(wizard.connectedPath, 15);
  } catch (e) {
    live.value = isNotImplemented(e) ? "unavailable" : "stopped";
  }
});
onBeforeUnmount(() => {
  offs.forEach((o) => o());
  offs = [];
  if (live.value !== "unavailable") void upcoming.rcChannelsStreamStop().catch(() => undefined);
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
          {{ t("switches.boxName", { name: sw.mode, version: fw || "?" }) }}
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
        <button v-if="modeKeyOf(sw.mode) !== 'arm'" type="button" class="btn btn--small btn--link" @click="remove(i)">{{ t("switches.remove") }}</button>
      </span>
    </div>

    <div v-if="addable.length" class="btn-row">
      <button v-for="k in addable" :key="k" type="button" class="btn btn--small" :data-add="k" @click="add(k)">+ {{ t(`switches.modes.${k}`) }}</button>
    </div>
    <p class="hint">{{ t("switches.armHint") }}</p>
  </div>
</template>
