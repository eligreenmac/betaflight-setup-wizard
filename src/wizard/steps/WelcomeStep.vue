<script setup lang="ts">
import { useI18n } from "vue-i18n";
defineProps<{ stepId: string }>();
const { t } = useI18n();
</script>

<template>
  <div>
    <h2>{{ t("steps.welcome.title") }}</h2>
    <p>{{ t("steps.welcome.body") }}</p>
    <p><strong>{{ t("common.propsOffWarning") }}</strong></p>
  </div>
</template>
