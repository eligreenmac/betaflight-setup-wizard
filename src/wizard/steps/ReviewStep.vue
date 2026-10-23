<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U6: read-only plan (config_plan), cleared ports first. Without config_plan only a
// config_generate preview is shown and Apply stays blocked (ARCHITECTURE §5.1).
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import ErrorPanel from "../../components/ErrorPanel.vue";
import type { CliScript } from "../../types/model";
import { goTo } from "../nav";
import { fetchPlan } from "../plan";
import { getRolePort, neededRoles } from "../ports";
import { isHdOsdPlanError, osdChoice, withOsd } from "../osd";
import { toUiError, uiError, type UiError } from "../../errors/codes";
import { ALIGNMENT_PRESETS, sameAlignment } from "../orientation";
import { simulatedFields, toPlanModel, unplannedChoices } from "../modelExt";
import { calibrationWarnings } from "../guards";
import { modeKeyOf } from "../switches";
import { wizard } from "../state";

defineProps<{ stepId: string }>();

/** config_plan refused an HD OSD: firmware without MSP DisplayPort. */
function isHdNoDisplayport(e: UiError): boolean {
  const reason = e.details && typeof e.details === "object" ? (e.details as { reason?: unknown }).reason : undefined;
  return reason === "OSD_HD_NO_DISPLAYPORT" || isHdOsdPlanError(e);
}
const { t } = useI18n();
const plan = ref<CliScript | null>(null);
const previewOnly = ref(false);
const error = ref<unknown>(null);
const loading = ref(false);
const m = computed(() => wizard.model);

async function load(): Promise<void> {
  loading.value = true;
  error.value = null;
  wizard.ui.script = null;
  wizard.ui.planUnavailable = false;
  plan.value = null;
  try {
    const r = await fetchPlan(withOsd(toPlanModel(wizard.model, wizard.ext), wizard.osd), wizard.ports);
    if (r.status === "ready") {
      plan.value = r.script;
      previewOnly.value = false;
      // Apply sends exactly this script.
      wizard.ui.script = r.script;
    } else {
      plan.value = r.preview;
      previewOnly.value = true;
      wizard.ui.planUnavailable = true;
    }
  } catch (e) {
    const ue = toUiError(e);
    // HD OSD on firmware without MSP DisplayPort (#78: CONFIG with details.reason
    // OSD_HD_NO_DISPLAYPORT): plain words and a way back to the OSD step. The message
    // match stays as a fallback for a backend from before #78.
    error.value = isHdNoDisplayport(ue) ? { ...uiError("OSD_HD_UNSUPPORTED", ue.message) } : e;
  } finally {
    loading.value = false;
  }
}
onMounted(load);

/** "Back to OSD settings": HD is off for this board until it is identified again. */
function backToOsd(): void {
  wizard.osd.hdRejected = true;
  if (wizard.osd.askedSystem === "HD") wizard.osd.askedSystem = null;
  goTo("osd");
}
function onError(id: string): void {
  if (id === "editComponents") goTo("components");
  else if (id === "backToOsd") backToOsd();
  else void load();
}

const rows = computed(() => {
  const r: [string, string][] = [
    [t("review.board"), `${wizard.fc?.boardName ?? "?"} · ${wizard.fc?.firmwareVersion ?? "?"}`],
    [t("review.receiver"), m.value.receiver.kind === "NONE" ? t("review.none") : `${t(`receiver.protocols.${m.value.receiver.protocol ?? "CRSF"}`)}`],
    [t("review.vtx"), m.value.vtx.kind === "NONE" ? t("review.none") : t(`components.vtxKind.${m.value.vtx.kind}`)],
    [t("review.gps"), m.value.gps.enabled ? t("review.yes") : t("review.none")],
    [t("review.esc"), t(`esc.protocols.${m.value.esc.protocol}`)],
  ];
  for (const role of neededRoles(wizard.model, wizard.ui)) r.push([t(`roles.${role}`), getRolePort(wizard.model, role) ?? "—"]);
  r.push([t("review.switches"), m.value.switches.map((s) => `${modeKeyOf(s.mode) ? t(`switches.modes.${modeKeyOf(s.mode)}`) : s.mode} AUX${s.auxChannel + 1}`).join(", ")]);
  const osd = osdChoice(wizard.osd, m.value.gps.enabled);
  if (m.value.vtx.kind !== "NONE")
    r.push([t("review.osd"), osd ? t("review.osdChosen", { n: osd.elements.filter((e) => e.enabled).length, system: t(`osd.system.${osd.videoSystem}`) }) : t("review.osdKeep")]);
  const x = wizard.ext;
  const skip = new Set(simulatedFields());
  const keep = t("review.keep");
  r.push([t("review.airmode"), x.airmode ? t(`switches.airmode.${x.airmode.toLowerCase()}`) : keep]);
  if (x.removeModes.length)
    r.push([t("review.removeModes"), x.removeModes.map((m) => { const k = modeKeyOf(m); return k ? t(`switches.modes.${k}`) : m; }).join(", ")]);
  const sensorParts = x.sensors && !skip.has("sensors")
    ? (["acc", "baro", "mag"] as const).filter((k) => x.sensors![k] !== null).map((k) => `${t(`calibration.sensors.${k}`)}: ${x.sensors![k] ? t("review.on") : t("review.off")}`)
    : [];
  r.push([t("review.sensors"), sensorParts.length ? sensorParts.join(" · ") : keep]);
  const preset = x.boardAlignment && ALIGNMENT_PRESETS.find((p) => sameAlignment(x.boardAlignment, p.align));
  r.push([t("review.alignment"), x.boardAlignment && !skip.has("boardAlignment") ? (preset ? t(`calibration.orientation.presets.${preset.id}`) : `${x.boardAlignment.roll}/${x.boardAlignment.pitch}/${x.boardAlignment.yaw}`) : keep]);
  r.push([t("review.accCalibration"), t(`review.calibration.${wizard.ui.accCalibration}`)]);
  r.push([t("review.beepers"), x.beepers ? (x.beepers.disabled.length ? t("review.beepersOff", { list: x.beepers.disabled.join(", ") }) : t("review.beepersAllOn")) : keep]);
  if (x.beepers?.dshotBeaconDisabled)
    r.push([t("review.beacon"), x.beepers.dshotBeaconDisabled.length ? t("review.beepersOff", { list: x.beepers.dshotBeaconDisabled.join(", ") }) : t("review.beepersAllOn")]);
  return r;
});
const unplanned = computed(() => (previewOnly.value ? [] : unplannedChoices(wizard.ext, plan.value)));
const warnings = computed(() => calibrationWarnings(wizard));
/** The OSD was set up, but this version's plan does not write it (engine without `osd`). */
const osdUnplanned = computed(() => !previewOnly.value && !!plan.value && !!osdChoice(wizard.osd, m.value.gps.enabled) && !plan.value.commands.some((c) => /^set osd_\w+_pos\b/.test(c)));
</script>

<template>
  <div>
    <h2>{{ t("steps.review.title") }}</h2>
    <p class="lead">{{ t("steps.review.description") }}</p>
    <p v-if="loading" class="hint">{{ t("review.loading") }}</p>
    <ErrorPanel v-if="error" :error="error" @action="onError" />

    <div v-if="previewOnly" class="callout callout--danger" data-test="plan-unavailable">
      <strong>{{ t("review.planUnavailableTitle") }}</strong>
      <p>{{ t("review.planUnavailableBody") }}</p>
    </div>

    <template v-if="plan">
      <section class="callout callout--warn" data-test="clearings">
        <h3>{{ t("review.clearingsTitle") }}</h3>
        <p v-if="previewOnly">{{ t("review.clearingsUnknown") }}</p>
        <p v-else-if="!plan.cleared.length">{{ t("review.noClearings") }}</p>
        <ul v-else>
          <li v-for="c in plan.cleared" :key="c.port">
            {{ t("review.clearing", { port: c.port, functions: c.functions.map((f) => t(`serialFunctions.${f}`)).join(", ") }) }}
          </li>
        </ul>
        <p class="hint">{{ t("review.clearingsHint") }}</p>
      </section>

      <h3>{{ t("review.summary") }}</h3>
      <dl class="kv">
        <template v-for="[k, v] in rows" :key="k"><dt>{{ k }}</dt><dd>{{ v }}</dd></template>
      </dl>

      <div v-if="osdUnplanned" class="callout callout--warn" data-test="osd-unplanned">
        <strong>{{ t("review.osdUnplannedTitle") }}</strong>
        <p>{{ t("review.osdUnplannedBody") }}</p>
      </div>

      <div v-for="w in warnings" :key="w.key" class="callout callout--warn" :data-warning="w.key">{{ t(w.key) }}</div>

      <div v-if="unplanned.length" class="callout callout--warn" data-test="unplanned">
        <strong>{{ t("review.unplannedTitle") }}</strong>
        <p>{{ t("review.unplannedBody", { list: unplanned.map((f) => t(`review.fields.${f}`)).join(", ") }) }}</p>
      </div>

      <div v-if="plan.warnings.length" class="callout callout--warn">
        <h3>{{ t("review.warnings") }}</h3>
        <ul><li v-for="w in plan.warnings" :key="w" dir="ltr">{{ w }}</li></ul>
      </div>

      <details>
        <summary>{{ t("review.commands", { n: plan.commands.length }) }}</summary>
        <pre dir="ltr" data-test="cli">{{ plan.commands.join("\n") }}</pre>
      </details>
      <p class="hint">{{ t("review.nothingWritten") }}</p>
    </template>
  </div>
</template>
