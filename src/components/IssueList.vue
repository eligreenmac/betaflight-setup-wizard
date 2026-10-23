<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
import { useI18n } from "vue-i18n";
import type { Issue } from "../wizard/guards";
import { rxProtocolLabel } from "../wizard/receiver";

defineProps<{ issues: Issue[] }>();
const { t, te } = useI18n();

/** `role` / `rxProtocol` params are ids; translate them so the sentence reads naturally. */
function text(i: Issue): string {
  const params = { ...(i.params ?? {}) };
  if (typeof params.role === "string") params.role = t(`roles.${params.role}`);
  if (typeof params.rxProtocol === "string") params.rxProtocol = rxProtocolLabel(params.rxProtocol, t, te);
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
