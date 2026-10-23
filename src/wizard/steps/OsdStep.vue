<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// OSD (on-screen display): what it is, which video system (detected from the VTX, or
// asked), a checklist of elements (defaults from config_osd_default_layout), and a
// preview on the board's own grid (any size). Elements can be dragged (clamped to the
// grid) or moved with the arrow keys.
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { osdApi, type OsdSystem } from "../../api/osdPending";
import ErrorPanel from "../../components/ErrorPanel.vue";
import { toUiError, type UiError } from "../../errors/codes";
import { clampPos, effectiveSystem, offeredElements, OSD_GROUPS, previewPos, recommendedEnabled, systemFromVtx, type OsdElementInfo } from "../osd";
import { markDirty, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const detected = computed(() => systemFromVtx(wizard.model.vtx));
const system = computed(() => effectiveSystem(wizard.model, wizard.osd));
const changing = ref(false);
const loading = ref(false);
const error = ref<UiError | null>(null);
const gps = computed(() => wizard.model.gps.enabled);
const offered = computed(() => offeredElements(wizard.osd.layout, gps.value));
const groups = computed(() => OSD_GROUPS.map((g) => ({ g, items: offered.value.filter((e) => e.group === g) })).filter((x) => x.items.length));
const grid = computed(() => wizard.osd.layout?.grid ?? null);
const shown = computed(() => offered.value.filter((e) => wizard.osd.enabled[e.name]));
const isRecommended = computed(() => {
  if (!wizard.osd.layout) return true;
  const rec = recommendedEnabled(wizard.osd.layout, gps.value);
  return Object.keys(rec).every((k) => !!wizard.osd.enabled[k] === rec[k]) && Object.keys(wizard.osd.moved).length === 0;
});

function setSystem(s: OsdSystem): void {
  wizard.osd.askedSystem = s;
  changing.value = false;
  markDirty();
}

async function loadLayout(): Promise<void> {
  const s = system.value;
  if (!s || (wizard.osd.layout && wizard.osd.layoutSystem === s)) return;
  loading.value = true;
  error.value = null;
  try {
    const layout = await osdApi.defaultLayout(s);
    if (system.value !== s) return;
    wizard.osd.layout = layout;
    wizard.osd.layoutSystem = s;
    // A new system means a new layout: start again from its recommendation.
    wizard.osd.enabled = recommendedEnabled(layout, gps.value);
    wizard.osd.moved = {};
  } catch (e) {
    error.value = toUiError(e);
  } finally {
    loading.value = false;
  }
}

function setMode(mode: "keep" | "choose"): void {
  wizard.osd.mode = mode;
  markDirty();
}
function toggle(name: string, on: boolean): void {
  wizard.osd.enabled = { ...wizard.osd.enabled, [name]: on };
  markDirty();
}
function reset(): void {
  if (!wizard.osd.layout) return;
  wizard.osd.enabled = recommendedEnabled(wizard.osd.layout, gps.value);
  wizard.osd.moved = {};
  markDirty();
}

// ---- preview: drag / arrow keys, always clamped to the grid
const canvas = ref<HTMLElement | null>(null);
const selected = ref<string | null>(null);
let dragging: { name: string; dx: number; dy: number } | null = null;
function moveTo(e: OsdElementInfo, x: number, y: number): void {
  if (!grid.value) return;
  wizard.osd.moved = { ...wizard.osd.moved, [e.name]: clampPos(x, y, grid.value, e.sample.length) };
  markDirty();
}
function cellAt(ev: PointerEvent): { x: number; y: number } | null {
  const r = canvas.value?.getBoundingClientRect();
  if (!r || !grid.value || !r.width || !r.height) return null;
  return { x: ((ev.clientX - r.left) / r.width) * grid.value.cols, y: ((ev.clientY - r.top) / r.height) * grid.value.rows };
}
function down(e: OsdElementInfo, ev: PointerEvent): void {
  selected.value = e.name;
  const c = cellAt(ev);
  const p = previewPos(wizard.osd, e.name);
  if (!c || !p) return;
  dragging = { name: e.name, dx: c.x - p.x, dy: c.y - p.y };
  (ev.target as HTMLElement).setPointerCapture?.(ev.pointerId);
}
function move(e: OsdElementInfo, ev: PointerEvent): void {
  if (!dragging || dragging.name !== e.name) return;
  const c = cellAt(ev);
  if (c) moveTo(e, Math.floor(c.x - dragging.dx + 0.5), Math.floor(c.y - dragging.dy + 0.5));
}
function up(): void {
  dragging = null;
}
function key(e: OsdElementInfo, ev: KeyboardEvent): void {
  const p = previewPos(wizard.osd, e.name);
  if (!p) return;
  const d: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  const v = d[ev.key];
  if (!v) return;
  ev.preventDefault();
  moveTo(e, p.x + v[0], p.y + v[1]);
}
function style(e: OsdElementInfo): Record<string, string> {
  const p = previewPos(wizard.osd, e.name)!;
  const g = grid.value!;
  return { left: `${(p.x / g.cols) * 100}%`, top: `${(p.y / g.rows) * 100}%`, width: `${(e.sample.length / g.cols) * 100}%`, height: `${(1 / g.rows) * 100}%` };
}
const sample = (e: OsdElementInfo) => (e.name === "craft_name" ? (wizard.model.extras?.craftName || e.sample).toUpperCase() : e.sample);

watch(system, () => void loadLayout());
onMounted(loadLayout);
</script>

<template>
  <div>
    <h2>{{ t("steps.osd.title") }}</h2>
    <p class="lead">{{ t("steps.osd.description") }}</p>
    <p class="hint">{{ t("osd.intro") }}</p>

    <!-- Video system -->
    <section class="osd-system" data-section="osd-system">
      <h3>{{ t("osd.system.title") }}</h3>
      <p v-if="detected && !changing && !wizard.osd.askedSystem" data-test="osd-detected">
        {{ t(`osd.system.detected.${detected}`) }}
        <button type="button" class="btn btn--small btn--link" data-test="osd-change-system" @click="changing = true">{{ t("osd.system.change") }}</button>
      </p>
      <template v-else>
        <p v-if="!detected">{{ t("osd.system.ask") }}</p>
        <div class="osd-choices" role="radiogroup" :aria-label="t('osd.system.title')">
          <label v-for="s in ['ANALOG', 'HD'] as const" :key="s">
            <input type="radio" name="osd-system" :data-osd-system="s" :checked="system === s" @change="setSystem(s)" />
            <span>
              <strong>{{ t(`osd.system.${s}`) }}</strong>
              <small class="hint" style="display: block">{{ t(`osd.system.${s}Hint`) }}</small>
            </span>
          </label>
        </div>
      </template>
    </section>

    <template v-if="system">
      <div class="osd-choices" role="radiogroup" :aria-label="t('steps.osd.title')">
        <label>
          <input type="radio" name="osd-mode" data-osd-mode="choose" :checked="wizard.osd.mode === 'choose'" @change="setMode('choose')" />
          <span>
            <strong>{{ t("osd.choose") }}</strong>
            <small class="hint" style="display: block">{{ t("osd.chooseHint") }}</small>
          </span>
        </label>
        <label>
          <input type="radio" name="osd-mode" data-osd-mode="keep" :checked="wizard.osd.mode === 'keep'" @change="setMode('keep')" />
          <span>
            <strong>{{ t("osd.keep") }}</strong>
            <small class="hint" style="display: block">{{ t("osd.keepHint") }}</small>
          </span>
        </label>
      </div>

      <p v-if="loading" class="hint" role="status">{{ t("osd.loading") }}</p>
      <template v-if="error">
        <ErrorPanel :error="error" @action="loadLayout" />
        <button type="button" class="btn btn--small" data-test="osd-retry" @click="loadLayout">{{ t("actions.tryAgain") }}</button>
      </template>

      <template v-if="wizard.osd.mode === 'choose' && wizard.osd.layout && grid">
        <div class="osd-layout">
          <div>
            <div class="btn-row">
              <button type="button" class="btn btn--small" data-test="osd-reset" :disabled="isRecommended" @click="reset">{{ t("osd.reset") }}</button>
              <span v-if="isRecommended" class="hint" data-test="osd-recommended">{{ t("osd.isRecommended") }}</span>
            </div>
            <section v-for="grp in groups" :key="grp.g" class="osd-group" :data-group="grp.g">
              <h3>{{ t(`osd.groups.${grp.g}`) }}</h3>
              <ul>
                <li v-for="e in grp.items" :key="e.name">
                  <label>
                    <input type="checkbox" :data-osd="e.name" :checked="!!wizard.osd.enabled[e.name]" @change="toggle(e.name, ($event.target as HTMLInputElement).checked)" />
                    <span>
                      <strong>{{ t(`osd.items.${e.name}.label`) }}</strong>
                      <small class="hint" style="display: block">{{ t(`osd.items.${e.name}.hint`) }}</small>
                    </span>
                  </label>
                </li>
              </ul>
            </section>
          </div>

          <figure class="osd-preview-wrap">
            <div
              ref="canvas"
              :class="['osd-preview', wizard.osd.layoutSystem === 'HD' ? 'osd-preview--hd' : 'osd-preview--analog']"
              dir="ltr"
              data-test="osd-preview"
              :data-cols="grid.cols"
              :data-rows="grid.rows"
              :style="{ '--osd-cols': String(grid.cols) }"
            >
              <span
                v-for="e in shown"
                :key="e.name"
                class="osd-el"
                :class="{ 'osd-el--selected': selected === e.name }"
                :data-preview="e.name"
                :style="style(e)"
                tabindex="0"
                role="button"
                :aria-label="t('osd.moveLabel', { item: t(`osd.items.${e.name}.label`) })"
                @pointerdown="down(e, $event)"
                @pointermove="move(e, $event)"
                @pointerup="up"
                @pointercancel="up"
                @focus="selected = e.name"
                @keydown="key(e, $event)"
                >{{ sample(e) }}</span
              >
            </div>
            <figcaption class="hint">{{ t("osd.previewHint", { cols: grid.cols, rows: grid.rows }) }}</figcaption>
          </figure>
        </div>
      </template>
    </template>
  </div>
</template>
