<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
import { useI18n } from "vue-i18n";
defineProps<{ stepId: string }>();
const { t } = useI18n();
</script>

<template>
  <div>
    <h2>{{ t("steps.welcome.title") }}</h2>
    <p class="lead">{{ t("steps.welcome.body") }}</p>
    <h3>{{ t("welcome.needTitle") }}</h3>
    <ul>
      <li>{{ t("welcome.need1") }}</li>
      <li>{{ t("welcome.need2") }}</li>
      <li>{{ t("welcome.need3") }}</li>
    </ul>
    <div class="callout callout--warn">
      <strong>{{ t("common.propsOffWarning") }}</strong>
    </div>
    <p class="hint">{{ t("welcome.time") }}</p>
  </div>
</template>
