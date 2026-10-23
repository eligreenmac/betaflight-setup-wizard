<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// Ordered stage list for long operations (firmware update, apply).
import { useI18n } from "vue-i18n";

const props = defineProps<{
  /** i18n keys, in order. */
  labels: string[];
  /** Index of the running stage; -1 = not started; labels.length = all done. */
  current: number;
  failed?: boolean;
  /** 0..1 progress of the current stage, if known. */
  fraction?: number | null;
}>();
const { t } = useI18n();

function state(i: number): "done" | "active" | "failed" | "pending" {
  if (i < props.current) return "done";
  if (i === props.current) return props.failed ? "failed" : "active";
  return "pending";
}
const icon = { done: "✓", active: "…", failed: "✗", pending: "○" } as const;
</script>

<template>
  <ol class="stages" aria-live="polite">
    <li v-for="(k, i) in labels" :key="k" :class="state(i)" :data-stage-state="state(i)">
      <span class="stages__icon" aria-hidden="true">{{ icon[state(i)] }}</span>
      <span>{{ t(k) }}</span>
      <progress v-if="state(i) === 'active' && fraction != null" :value="fraction" max="1" />
      <span class="visually-hidden" v-if="state(i) !== 'pending'">({{ t(`stageState.${state(i)}`) }})</span>
    </li>
  </ol>
</template>

<style scoped>
.visually-hidden { position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip-path: inset(50%); }
</style>
