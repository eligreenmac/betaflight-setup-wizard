<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { setLocale, type Locale } from "./i18n";
import { visibleSteps } from "./wizard/engine";
import { resolveStepComponent } from "./wizard/registry";
import { wizard } from "./wizard/state";

const { t, locale } = useI18n();
const steps = computed(() => visibleSteps(wizard.model));
const index = ref(0);
const current = computed(() => steps.value[Math.min(index.value, steps.value.length - 1)]);

function onLocale(e: Event) {
  setLocale((e.target as HTMLSelectElement).value as Locale);
}
</script>

<template>
  <main class="app">
    <header class="toolbar">
      <h1>{{ t("app.title") }}</h1>
      <label>
        {{ t("common.language") }}
        <select :value="locale" @change="onLocale">
          <option value="he">עברית</option>
          <option value="en">English</option>
        </select>
      </label>
    </header>

    <ol class="steps">
      <li v-for="(s, i) in steps" :key="s.id" :class="{ active: i === index }">{{ t(s.titleKey) }}</li>
    </ol>

    <section class="card">
      <component :is="resolveStepComponent(current.component)" :step-id="current.id" />
    </section>

    <nav class="nav">
      <button :disabled="index === 0" @click="index--">{{ t("common.back") }}</button>
      <button class="primary" :disabled="index >= steps.length - 1" @click="index++">{{ t("common.next") }}</button>
    </nav>
  </main>
</template>
