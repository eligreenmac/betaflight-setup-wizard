<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U6: done: what changed, what to do next, backups, start over.
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { openExternal } from "../../api/links";
import BackupsPanel from "../../components/BackupsPanel.vue";
import { motorTest } from "../motorTest";
import { resetNav } from "../nav";
import { resetWizard, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();
const fallback = ref<string | null>(null);
const LINKS = [
  { key: "done.linkDocs", url: "https://betaflight.com/docs/wiki" },
  { key: "done.linkIssues", url: "https://github.com/eligreenmac/betaflight-setup-wizard/issues" },
];

async function open(url: string): Promise<void> {
  const r = await openExternal(url);
  fallback.value = r.opened ? null : r.target;
}
async function startOver(): Promise<void> {
  await motorTest.stop("navigation");
  resetWizard();
  resetNav();
}
</script>

<template>
  <div>
    <h2>{{ t("steps.done.title") }}</h2>
    <p class="lead">
      {{ wizard.ui.applyStatus === "verified" ? t("done.success", { board: wizard.fc?.boardName ?? "" }) : t("done.notApplied") }}
    </p>
    <h3>{{ t("done.nextTitle") }}</h3>
    <ol>
      <li>{{ t("done.next1") }}</li>
      <li>{{ t("done.next2") }}</li>
      <li>{{ t("done.next3") }}</li>
    </ol>
    <p class="callout callout--warn">{{ t("common.propsOffWarning") }}</p>
    <h3>{{ t("done.help") }}</h3>
    <ul>
      <li v-for="l in LINKS" :key="l.url">
        <a :href="l.url" target="_blank" rel="noopener noreferrer" @click.prevent="open(l.url)">{{ t(l.key) }}</a>
      </li>
    </ul>
    <p v-if="fallback" class="hint">{{ t("links.fallback") }} <code dir="ltr">{{ fallback }}</code></p>
    <BackupsPanel v-if="wizard.fc" />
    <div class="btn-row"><button type="button" class="btn" data-test="start-over" @click="startOver">{{ t("done.startOver") }}</button></div>
  </div>
</template>
