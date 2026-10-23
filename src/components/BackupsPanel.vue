<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U6: list this board's backups, open the backups folder, restore one (config_restore) after confirmation.
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { openBackupsFolder } from "../api/links";
import type { Backup } from "../types/model";
import { canRestoreOnto, listBackups, restoreBackup } from "../wizard/backups";
import { setBoard, wizard } from "../wizard/state";
import ErrorPanel from "./ErrorPanel.vue";

const { t, locale } = useI18n();
const backups = ref<Backup[]>([]);
const source = ref<"backend" | "session">("session");
const loading = ref(false);
const confirming = ref<Backup | null>(null);
const restoring = ref(false);
const restored = ref<Backup | null>(null);
const error = ref<unknown>(null);
const folderPath = ref<string | null>(null);

async function load(): Promise<void> {
  loading.value = true;
  error.value = null;
  try {
    const r = await listBackups(wizard.fc?.uid, wizard.ui.sessionBackups);
    backups.value = r.backups;
    source.value = r.source;
  } catch (e) {
    error.value = e;
  } finally {
    loading.value = false;
  }
}

async function openFolder(): Promise<void> {
  const r = await openBackupsFolder(wizard.fc?.uid ?? null);
  folderPath.value = r.opened ? null : r.target;
}

async function restore(b: Backup): Promise<void> {
  restoring.value = true;
  error.value = null;
  try {
    const res = await restoreBackup(b);
    // The backend takes a safety backup before restoring.
    if (res.result.backup) wizard.ui.sessionBackups.unshift(res.result.backup);
    if (wizard.fc) setBoard(wizard, wizard.fc, res.ports, wizard.connectedPath);
    restored.value = b;
    confirming.value = null;
    void load();
  } catch (e) {
    error.value = e;
  } finally {
    restoring.value = false;
  }
}

function when(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString(locale.value, { dateStyle: "medium", timeStyle: "short" });
}

onMounted(load);
defineExpose({ load });
</script>

<template>
  <section class="card backups" data-test="backups">
    <h3>{{ t("backups.title") }}</h3>
    <p class="hint">{{ t(source === "backend" ? "backups.intro" : "backups.sessionOnly") }}</p>
    <div class="btn-row">
      <button type="button" class="btn btn--small" data-test="open-folder" @click="openFolder">{{ t("backups.openFolder") }}</button>
      <button type="button" class="btn btn--small btn--link" :disabled="loading" @click="load">{{ t("backups.refresh") }}</button>
    </div>
    <p v-if="folderPath" class="callout callout--info" data-test="folder-path">
      {{ t("backups.folderFallback") }} <code dir="ltr">{{ folderPath }}</code>
    </p>

    <p v-if="loading" class="hint">{{ t("backups.loading") }}</p>
    <p v-else-if="!backups.length" class="hint">{{ t("backups.none") }}</p>
    <ul v-else>
      <li v-for="b in backups" :key="b.id">
        <span>{{ t("backups.item", { board: b.boardName, version: b.firmwareVersion, date: when(b.createdAt) }) }}</span>
        <button
          v-if="canRestoreOnto(b, wizard.fc?.uid)"
          type="button"
          class="btn btn--small"
          :disabled="restoring"
          data-test="restore"
          @click="confirming = b"
        >
          {{ t("backups.restore") }}
        </button>
        <small v-else class="hint">{{ t("backups.otherBoard") }}</small>
      </li>
    </ul>

    <div v-if="confirming" class="callout callout--warn" role="alertdialog" data-test="restore-confirm">
      <p><strong>{{ t("backups.confirmTitle") }}</strong></p>
      <p>{{ t("backups.confirmBody", { date: when(confirming.createdAt) }) }}</p>
      <div class="btn-row">
        <button type="button" class="btn btn--danger" :disabled="restoring" data-test="restore-yes" @click="restore(confirming)">
          {{ restoring ? t("backups.restoring") : t("backups.confirmYes") }}
        </button>
        <button type="button" class="btn" :disabled="restoring" @click="confirming = null">{{ t("backups.confirmNo") }}</button>
      </div>
    </div>
    <p v-if="restored" class="callout callout--ok" data-test="restored">{{ t("backups.restored", { date: when(restored.createdAt) }) }}</p>
    <ErrorPanel v-if="error" :error="error" @action="load" />
  </section>
</template>
