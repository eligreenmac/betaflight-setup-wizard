<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
import { useI18n } from "vue-i18n";
import { canGoTo } from "../wizard/guards";
import { currentIndex, currentStep, goTo, progress, steps } from "../wizard/nav";
import { wizard } from "../wizard/state";

const { t } = useI18n();
</script>

<template>
  <nav :aria-label="t('shell.steps')">
    <ol class="stepper">
      <li v-for="(s, i) in steps" :key="s.id" :class="{ current: i === currentIndex, done: i < currentIndex }">
        <button
          type="button"
          class="stepper__btn"
          :aria-current="i === currentIndex ? 'step' : undefined"
          :disabled="i === currentIndex || !canGoTo(s.id, currentStep.id, wizard, steps)"
          @click="goTo(s.id)"
        >
          <span class="stepper__num">{{ i < currentIndex ? "✓" : i + 1 }}</span>
          <span>{{ t(s.titleKey) }}</span>
        </button>
      </li>
    </ol>
    <div class="progress" role="progressbar" :aria-label="t('shell.progress')" aria-valuemin="0" aria-valuemax="100" :aria-valuenow="progress">
      <div class="progress__bar" :style="{ inlineSize: `${progress}%` }" />
    </div>
  </nav>
</template>
