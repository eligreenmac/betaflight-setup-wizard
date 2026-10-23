<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// AppError.code -> guidance (src/errors/guidance.ts). Link actions open in the system
// browser via the opener plugin; every other action is emitted to the parent step.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { openExternal } from "../api/links";
import { guidanceFor, toUiError, type GuidanceAction, type GuidanceActionId } from "../errors/guidance";

const props = defineProps<{ error: unknown; busy?: boolean; hide?: GuidanceActionId[] }>();
const emit = defineEmits<{ action: [id: GuidanceActionId] }>();
const { t } = useI18n();

const err = computed(() => toUiError(props.error));
const g = computed(() => guidanceFor(err.value));
const actions = computed(() => g.value.actions.filter((a) => !props.hide?.includes(a.id)));
const linkError = ref<string | null>(null);

async function run(a: GuidanceAction) {
  linkError.value = null;
  if (a.id === "openLink" && a.href) {
    const r = await openExternal(a.href);
    if (!r.opened) linkError.value = r.target;
    return;
  }
  emit("action", a.id);
}
</script>

<template>
  <section :class="['callout', g.severity === 'warning' ? 'callout--warn' : 'callout--danger']" role="alert" :data-error-code="g.code">
    <h3>{{ t(g.titleKey) }}</h3>
    <p>{{ t(g.bodyKey) }}</p>
    <p v-if="g.showMessage && err.message" class="mono" dir="ltr">{{ err.message }}</p>
    <ol>
      <li v-for="k in g.stepKeys" :key="k">{{ t(k) }}</li>
    </ol>
    <div class="btn-row">
      <button
        v-for="a in actions"
        :key="a.id + a.labelKey"
        type="button"
        :class="['btn', a.primary ? 'btn--primary' : '']"
        :disabled="busy"
        @click="run(a)"
      >
        {{ t(a.labelKey) }}
      </button>
    </div>
    <p v-if="linkError" class="hint">
      {{ t("links.fallback") }}
      <a :href="linkError" target="_blank" rel="noopener noreferrer"><bdi class="mono">{{ linkError }}</bdi></a>
    </p>
    <details>
      <summary>{{ t("actions.technicalDetails") }}</summary>
      <pre>{{ err.code }} ({{ err.category }}): {{ err.message }}</pre>
    </details>
  </section>
</template>
