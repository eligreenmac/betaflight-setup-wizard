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
import { modeKeyOf } from "../switches";
import { wizard } from "../state";

defineProps<{ stepId: string }>();
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
    const r = await fetchPlan(wizard.model, wizard.ports);
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
    error.value = e;
  } finally {
    loading.value = false;
  }
}
onMounted(load);

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
  return r;
});
</script>

<template>
  <div>
    <h2>{{ t("steps.review.title") }}</h2>
    <p class="lead">{{ t("steps.review.description") }}</p>
    <p v-if="loading" class="hint">{{ t("review.loading") }}</p>
    <ErrorPanel v-if="error" :error="error" @action="(id) => (id === 'editComponents' ? goTo('components') : load())" />

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
