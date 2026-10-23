<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// UART picker for one wizard role. Lists ONLY the port names the board reports
// (never derived from identifiers, which changed in 2025.12: UART1 = 51; 2026.6 adds UART11..15 = 61..65).
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { excludedPortsFor, foreignBits, freePortsFor, functionNames, getRolePort, managedBits, setRolePort, type PortRole } from "../wizard/ports";
import { markDirty, wizard } from "../wizard/state";

const props = defineProps<{ role: PortRole }>();
const { t } = useI18n();

const free = computed(() => freePortsFor(props.role, wizard.ports, wizard.model, wizard.ui));
const excluded = computed(() => excludedPortsFor(props.role, wizard.ports, wizard.model, wizard.ui));
const id = computed(() => `port-${props.role}`);
const value = computed({
  get: () => getRolePort(wizard.model, props.role) ?? "",
  set: (v: string) => {
    setRolePort(wizard.model, props.role, v || null);
    markDirty();
  },
});

function current(mask: number): string {
  const names = functionNames(managedBits(mask));
  return names.length ? t("ports.currently", { functions: names.join(", ") }) : "";
}
function reason(r: { port: { functionMask: number }; reason: string }): string {
  return t(`ports.reason.${r.reason}`, { functions: functionNames(foreignBits(r.port.functionMask)).join(", ") });
}
</script>

<template>
  <div class="field">
    <label :for="id">{{ t(`ports.question.${role}`) }}</label>
    <select v-if="free.length" :id="id" v-model="value" :data-role="role">
      <option value="">{{ t("ports.choose") }}</option>
      <option v-for="p in free" :key="p.name" :value="p.name">{{ p.name }}{{ current(p.functionMask) ? ` — ${current(p.functionMask)}` : "" }}</option>
    </select>
    <p v-else class="callout callout--warn">{{ t("ports.noneFree") }}</p>
    <p class="hint">{{ t(`ports.wiring.${role}`) }}</p>
    <details v-if="excluded.length">
      <summary>{{ t("ports.whyMissing") }}</summary>
      <ul>
        <li v-for="x in excluded" :key="x.port.name"><bdi class="mono">{{ x.port.name }}</bdi>: {{ reason(x) }}</li>
      </ul>
    </details>
  </div>
</template>
