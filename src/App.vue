<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U1: app shell. Navigation comes from steps.json (wizard/nav), Next is gated by the
// step guards, and the motor test is stopped by every global stop trigger.
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { onAppCloseRequested } from "./api/window";
import BackupsPanel from "./components/BackupsPanel.vue";
import IssueList from "./components/IssueList.vue";
import LanguageToggle from "./components/LanguageToggle.vue";
import WizardStepper from "./components/WizardStepper.vue";
import { bindMotorStopTriggers, motorTest } from "./wizard/motorTest";
import { back, currentIndex, currentStep, isFirst, isLast, issues, nav, next, steps } from "./wizard/nav";
import { resolveStepComponent } from "./wizard/registry";
import { markDirty, wizard } from "./wizard/state";

const props = withDefaults(defineProps<{ demo?: boolean }>(), { demo: false });
const { t } = useI18n();
const showBackups = ref(false);
let unbind: (() => void) | null = null;

// Any change to the model after Review invalidates the reviewed plan / apply result.
watch(() => wizard.model, () => markDirty(), { deep: true });

onMounted(() => {
  unbind = bindMotorStopTriggers(motorTest, {
    target: window,
    navigation: () => nav.currentId,
    onAppClose: onAppCloseRequested,
    backendEvents: true,
  });
});
onBeforeUnmount(() => unbind?.());
</script>

<template>
  <div class="app">
    <header class="toolbar">
      <h1>{{ t("app.title") }}</h1>
      <span v-if="props.demo" class="badge" data-test="demo-badge">{{ t("shell.demo") }}</span>
      <span style="flex: 1" />
      <button v-if="wizard.fc" type="button" class="btn btn--small btn--link" :aria-expanded="showBackups" @click="showBackups = !showBackups">
        {{ t("backups.title") }}
      </button>
      <LanguageToggle />
    </header>

    <WizardStepper />

    <BackupsPanel v-if="showBackups && wizard.fc" />

    <main class="card">
      <component :is="resolveStepComponent(currentStep.component)" :key="currentStep.id" :step-id="currentStep.id" />
      <IssueList v-if="nav.attemptedNext && issues.length" :issues="issues" />
    </main>

    <nav class="btn-row nav" :aria-label="t('shell.navigation')">
      <button type="button" class="btn" :disabled="isFirst" data-test="back" @click="back()">{{ t("common.back") }}</button>
      <span class="hint">{{ t("shell.stepOf", { n: currentIndex + 1, total: steps.length }) }}</span>
      <button v-if="!isLast" type="button" class="btn btn--primary" data-test="next" @click="next()">{{ t("common.next") }}</button>
    </nav>
  </div>
</template>
