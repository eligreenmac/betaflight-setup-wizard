<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U6: apply (BACKUP/SENDING/SAVING/REBOOTING/RECONNECTED) then verify; mismatches as a table.
import { computed, onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, EVENTS } from "../../api/commands";
import { onEvent, type Unlisten } from "../../api/events";
import { APPLY_STAGES } from "../plan";
import BackupsPanel from "../../components/BackupsPanel.vue";
import ErrorPanel from "../../components/ErrorPanel.vue";
import StageList from "../../components/StageList.vue";
import { wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();
const stage = ref(-1);
const fraction = ref<number | null>(null);
const error = ref<unknown>(null);
const ui = computed(() => wizard.ui);
const busy = computed(() => ["applying", "verifying"].includes(ui.value.applyStatus));
let off: Unlisten | null = null;

async function apply(): Promise<void> {
  if (!wizard.ui.script || busy.value) return;
  error.value = null;
  stage.value = -1;
  wizard.ui.applyResult = null;
  wizard.ui.verify = null;
  wizard.ui.applyStatus = "applying";
  off?.();
  off = await onEvent(EVENTS.configProgress, (p) => {
    stage.value = APPLY_STAGES.indexOf(p.stage);
    fraction.value = p.fraction;
  });
  try {
    // Exactly the config_plan script from Review (never a config_generate preview).
    const res = await api.config.apply(wizard.ui.script);
    wizard.ui.applyResult = res;
    if (res.backup && !wizard.ui.sessionBackups.some((b) => b.id === res.backup!.id)) wizard.ui.sessionBackups.unshift(res.backup);
    stage.value = APPLY_STAGES.length;
    if (res.errors.length || !res.saved) {
      wizard.ui.applyStatus = "failed";
      return;
    }
    wizard.ui.applyStatus = "verifying";
    const v = await api.config.verify(wizard.model);
    wizard.ui.verify = v;
    wizard.ui.applyStatus = v.ok && v.mismatches.length === 0 ? "verified" : "mismatch";
  } catch (e) {
    error.value = e;
    wizard.ui.applyStatus = "failed";
  } finally {
    off?.();
    off = null;
  }
}
onBeforeUnmount(() => off?.());
</script>

<template>
  <div>
    <h2>{{ t("steps.apply.title") }}</h2>
    <p class="lead">{{ t("steps.apply.description") }}</p>

    <div v-if="!ui.script" class="callout callout--warn">{{ t("apply.noScript") }}</div>
    <template v-else>
      <p v-if="ui.applyStatus === 'idle'" class="hint">{{ t("apply.ready", { n: ui.script.commands.length }) }}</p>
      <p v-if="busy" class="callout callout--warn">{{ t("apply.doNotUnplug") }}</p>
      <StageList
        v-if="ui.applyStatus !== 'idle'"
        :labels="APPLY_STAGES.map((s) => `apply.stages.${s}`)"
        :current="stage"
        :failed="ui.applyStatus === 'failed' && stage < APPLY_STAGES.length"
        :fraction="fraction"
      />
      <p v-if="ui.applyStatus === 'verifying'" class="hint">{{ t("apply.verifying") }}</p>

      <div class="btn-row">
        <button v-if="ui.applyStatus === 'idle'" type="button" class="btn btn--primary" data-test="apply" @click="apply">{{ t("apply.apply") }}</button>
      </div>

      <p v-if="ui.applyResult && ui.applyResult.saved && !ui.applyResult.reconnected" class="callout callout--warn" data-test="not-reconnected">
        {{ t("apply.notReconnected") }}
      </p>
      <div v-if="ui.applyStatus === 'verified'" class="callout callout--ok" data-test="verified">
        <strong>{{ t("apply.verified") }}</strong>
        <p>{{ t("apply.verifiedBody", { n: ui.applyResult?.applied ?? 0 }) }}</p>
      </div>

      <div v-if="ui.applyStatus === 'mismatch' && ui.verify" class="callout callout--warn" data-test="mismatches">
        <h3>{{ t("apply.mismatch.title") }}</h3>
        <p>{{ t("apply.mismatch.body") }}</p>
        <table class="table">
          <thead><tr><th>{{ t("apply.mismatch.setting") }}</th><th>{{ t("apply.mismatch.expected") }}</th><th>{{ t("apply.mismatch.actual") }}</th></tr></thead>
          <tbody>
            <tr v-for="mm in ui.verify.mismatches" :key="mm.setting"><td dir="ltr">{{ mm.setting }}</td><td dir="ltr">{{ mm.expected }}</td><td dir="ltr">{{ mm.actual }}</td></tr>
          </tbody>
        </table>
        <ol>
          <li>{{ t("apply.mismatch.step1") }}</li>
          <li>{{ t("apply.mismatch.step2") }}</li>
          <li>{{ t("apply.mismatch.step3") }}</li>
        </ol>
        <button type="button" class="btn btn--primary" data-test="reapply" @click="apply">{{ t("apply.reapply") }}</button>
      </div>

      <div v-if="ui.applyStatus === 'failed'">
        <div v-if="ui.applyResult && ui.applyResult.errors.length" class="callout callout--danger" data-test="cli-errors">
          <h3>{{ t("apply.cliErrors") }}</h3>
          <table class="table">
            <thead><tr><th>{{ t("apply.command") }}</th><th>{{ t("apply.response") }}</th></tr></thead>
            <tbody><tr v-for="(e, i) in ui.applyResult.errors" :key="i"><td dir="ltr">{{ e.command }}</td><td dir="ltr">{{ e.response }}</td></tr></tbody>
          </table>
        </div>
        <p v-else-if="ui.applyResult && !ui.applyResult.saved" class="callout callout--danger">{{ t("apply.notSaved") }}</p>
        <ErrorPanel v-if="error" :error="error" @action="apply" />
        <p class="hint">{{ t("apply.failedHint") }}</p>
        <div class="btn-row"><button type="button" class="btn" @click="apply">{{ t("actions.tryAgain") }}</button></div>
      </div>
    </template>

    <!-- Outside the script block: a restore re-reads the board, which marks the plan stale. -->
    <details v-if="wizard.fc" :open="ui.applyStatus === 'failed' || ui.applyStatus === 'mismatch'">
      <summary>{{ t("apply.restoreTitle") }}</summary>
      <BackupsPanel />
    </details>
  </div>
</template>
