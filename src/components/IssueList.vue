<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
import { useI18n } from "vue-i18n";
import type { Issue } from "../wizard/guards";

defineProps<{ issues: Issue[] }>();
const { t } = useI18n();

/** `role` params are role ids; translate them so the sentence reads naturally. */
function text(i: Issue): string {
  const params = { ...(i.params ?? {}) };
  if (typeof params.role === "string") params.role = t(`roles.${params.role}`);
  return t(i.key, params);
}
</script>

<template>
  <div v-if="issues.length" class="callout callout--warn" role="alert">
    <h3>{{ t("shell.fixBeforeNext") }}</h3>
    <ul>
      <li v-for="i in issues" :key="i.key + JSON.stringify(i.params ?? {})">{{ text(i) }}</li>
    </ul>
  </div>
</template>
