<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// Persistent "Demo mode" banner: shown as soon as any call ran on the mock backend.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { demoState, isDemoMode } from "../api/fallback";

const { t } = useI18n();
const show = computed(() => isDemoMode());
</script>

<template>
  <div v-if="show" class="callout callout--warn demo-banner" role="status" data-test="demo-banner">
    <strong>{{ t("demo.title") }}</strong>
    {{ demoState.forced ? t("demo.bodyAll") : t("demo.bodySome") }}
    <details v-if="!demoState.forced && demoState.simulated.length" class="hint">
      <summary>{{ t("demo.which", { n: demoState.simulated.length }) }}</summary>
      <code dir="ltr">{{ demoState.simulated.join(", ") }}</code>
    </details>
  </div>
</template>
