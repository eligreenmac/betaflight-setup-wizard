<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
import { useI18n } from "vue-i18n";
import { setLocale, SUPPORTED_LOCALES, type Locale } from "../i18n";

const { t, locale } = useI18n();
// Each language is named in itself, so it is findable whatever the current UI language is.
const names: Record<Locale, string> = { he: "עברית", en: "English" };
</script>

<template>
  <div class="lang btn-row" role="group" :aria-label="t('shell.language')" style="margin: 0">
    <button
      v-for="l in SUPPORTED_LOCALES"
      :key="l"
      type="button"
      class="btn btn--small"
      :lang="l"
      :aria-pressed="locale === l"
      :data-locale="l"
      @click="setLocale(l)"
    >
      {{ names[l] }}
    </button>
  </div>
</template>
