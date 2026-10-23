<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
withDefaults(defineProps<{ tone?: "info" | "ok" | "warn" | "danger"; title?: string }>(), { tone: "info", title: undefined });
</script>

<template>
  <section :class="['callout', `callout--${tone}`]">
    <h3 v-if="title">{{ title }}</h3>
    <slot />
  </section>
</template>
