<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// After a firmware update with "Keep my settings" (wizard/restore.ts): what was put back,
// what the new firmware refused (verbatim), what was left out and why, and the restore's
// notes, each as a collapsible list. A refusal or failure reads "the firmware is installed,
// your settings were not put back, your backup is saved", with the Backups list right here.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { UiError } from "../errors/codes";
import { canRetryRestore, refusalKind, refusalStage, visibleSkips, type RestoreReport } from "../wizard/restore";
import BackupsPanel from "./BackupsPanel.vue";

const props = defineProps<{
  report: RestoreReport | null;
  error: UiError | null;
  backupId: string | null;
  busy?: boolean;
}>();
const emit = defineEmits<{ retry: [] }>();
const { t, te } = useI18n();
const showBackups = ref(false);

const skips = computed(() => (props.report ? visibleSkips(props.report) : []));
const kind = computed(() => (props.error ? refusalKind(props.error) : null));
const stage = computed(() => (props.error ? refusalStage(props.error) : null));
const retryable = computed(() => !!props.error && !!props.backupId && canRetryRestore(props.error));
const skipReason = (reason: string, detail: string) => (te(`restore.skip.${reason}`) ? t(`restore.skip.${reason}`) : detail);
</script>

<template>
  <div class="restore-result">
    <template v-if="report">
      <div class="callout" :class="report.saved && !report.failed.length ? 'callout--ok' : 'callout--warn'" data-test="restore-done">
        <h3>{{ t("restore.doneTitle") }}</h3>
        <p>{{ t("restore.doneBody", { n: report.applied }) }}</p>
        <p v-if="!report.saved" data-test="restore-not-saved">{{ t("restore.notSaved") }}</p>
        <p v-if="report.lineChanged" data-test="restore-line-changed">
          {{ t("restore.lineChanged", { from: report.fromVersion, to: report.toVersion }) }}
        </p>
      </div>
      <details v-if="report.failed.length" class="restore-list" data-test="restore-failed">
        <summary>{{ t("restore.failedTitle", { n: report.failed.length }) }}</summary>
        <p class="hint">{{ t("restore.failedHelp") }}</p>
        <ul>
          <li v-for="(f, i) in report.failed" :key="i">
            <code dir="ltr">{{ f.line }}</code>
            <span class="restore-list__why" dir="ltr">{{ f.error }}</span>
          </li>
        </ul>
      </details>
      <details v-if="skips.length" class="restore-list" data-test="restore-skipped">
        <summary>{{ t("restore.skippedTitle", { n: skips.length }) }}</summary>
        <p class="hint">{{ t("restore.skippedHelp") }}</p>
        <ul>
          <li v-for="(s, i) in skips" :key="i" :data-reason="s.reason">
            <code dir="ltr">{{ s.line }}</code>
            <span class="restore-list__why">{{ skipReason(s.reason, s.detail) }}</span>
          </li>
        </ul>
      </details>
      <details v-if="report.warnings.length" class="restore-list" data-test="restore-warnings">
        <summary>{{ t("restore.warningsTitle", { n: report.warnings.length }) }}</summary>
        <p class="hint">{{ t("restore.warningsHelp") }}</p>
        <ul>
          <li v-for="(w, i) in report.warnings" :key="i"><bdi dir="ltr">{{ w }}</bdi></li>
        </ul>
      </details>
    </template>

    <div v-else-if="error && kind" class="callout callout--warn" role="status" data-test="restore-refused" :data-kind="kind">
      <h3>{{ t("restore.refusedTitle") }}</h3>
      <p>{{ t(`restore.refused.${kind}`) }}</p>
      <p v-if="stage" data-test="restore-refused-stage">{{ t(`restore.refusedStage.${stage}`) }}</p>
      <p v-if="backupId" data-test="restore-backup-saved">{{ t("restore.backupSaved") }} <bdi dir="ltr">{{ backupId }}</bdi></p>
      <details class="restore-list">
        <summary>{{ t("restore.technical") }}</summary>
        <p dir="ltr"><code>{{ error.code }}</code> {{ error.message }}</p>
      </details>
      <div class="btn-row">
        <button v-if="retryable" type="button" class="btn btn--primary" :disabled="busy" data-test="restore-retry" @click="emit('retry')">{{ t("restore.retry") }}</button>
        <button type="button" class="btn" :aria-expanded="showBackups" data-test="restore-show-backups" @click="showBackups = !showBackups">
          {{ t(showBackups ? "restore.hideBackups" : "restore.showBackups") }}
        </button>
      </div>
      <BackupsPanel v-if="showBackups" />
    </div>

    <p v-else class="hint" data-test="restore-no-backup">{{ t("restore.noBackup") }}</p>
  </div>
</template>
