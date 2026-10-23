<script setup lang="ts">
// Placeholder used until the `ui` workstream implements each step.
import { useI18n } from "vue-i18n";
defineProps<{ stepId: string }>();
const { t } = useI18n();
</script>

<template>
  <div>
    <h2>{{ t(`steps.${stepId}.title`) }}</h2>
    <p>{{ t("common.notImplemented") }}</p>
  </div>
</template>
