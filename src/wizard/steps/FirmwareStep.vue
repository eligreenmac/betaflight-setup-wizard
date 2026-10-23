<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U3: release picker for the connected board's target, progress on flasher://progress.
// TODO(shared): target search filtered by MCU needs flasher_list_targets (stub today);
// the wizard uses the target the board reports, which covers the normal case.
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api } from "../../api/commands";
import ErrorPanel from "../../components/ErrorPanel.vue";
import ReadProtectedPanel from "../../components/ReadProtectedPanel.vue";
import StageList from "../../components/StageList.vue";
import { isNotImplemented, toUiError, type UiError } from "../../errors/codes";
import type { GuidanceActionId } from "../../errors/guidance";
import type { FirmwareRelease, FlashProgress } from "../../types/model";
import { runFirmwareUpdate, UPDATE_STAGES, type UpdateStage } from "../flash";
import { setBoard, wizard } from "../state";
import { compareVersions, isSupportedFirmware, MIN_SUPPORTED_FIRMWARE } from "../version";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const releases = ref<FirmwareRelease[]>([]);
const releasesError = ref<UiError | null>(null);
const chosen = ref<string>("");
const phase = ref<"idle" | "running" | "done" | "error">("idle");
const stage = ref<UpdateStage | null>(null);
const progress = ref<FlashProgress | null>(null);
const error = ref<UiError | null>(null);
const showPre = ref(false);
/** DFU device of the last attempt (flasher_remove_read_protection needs its serial). */
const dfuSerial = ref<string | null>(null);
const backupThisRun = ref(false);
const readProtected = computed(() => error.value?.code === "READ_PROTECTED");

const fc = computed(() => wizard.fc);
const supported = computed(() => !!fc.value && isSupportedFirmware(fc.value.fcVariant, fc.value.firmwareVersion));
const visibleReleases = computed(() => releases.value.filter((r) => showPre.value || !r.prerelease));
const latest = computed(() => wizard.ui.latestRelease);
const upToDate = computed(() => !!fc.value && !!latest.value && fc.value.fcVariant === "BTFL" && compareVersions(fc.value.firmwareVersion, latest.value) >= 0);
const stageIndex = computed(() => (phase.value === "done" ? UPDATE_STAGES.length : stage.value ? UPDATE_STAGES.indexOf(stage.value) : -1));

async function loadReleases(): Promise<void> {
  if (!fc.value) return;
  releasesError.value = null;
  try {
    const list = await api.flasher.listReleases(fc.value.boardName);
    releases.value = [...list].sort((a, b) => compareVersions(b.release, a.release));
    const stable = releases.value.find((r) => !r.prerelease);
    wizard.ui.latestRelease = stable?.release ?? null;
    chosen.value = stable?.release ?? releases.value[0]?.release ?? "";
  } catch (e) {
    releasesError.value = toUiError(e);
  }
}

function keep(): void {
  wizard.ui.firmwareDecision = "keep";
}

async function update(fullErase = false): Promise<void> {
  if (!fc.value || !chosen.value) return;
  phase.value = "running";
  error.value = null;
  wizard.ui.firmwareDecision = "update";
  try {
    const r = await runFirmwareUpdate(
      fc.value,
      chosen.value,
      {
        onStage: (s) => (stage.value = s),
        onProgress: (p) => (progress.value = p),
        onDfu: (d) => (dfuSerial.value = d.serial),
        onBackup: (b) => {
          wizard.ui.sessionBackups.unshift(b);
          backupThisRun.value = true;
        },
      },
      undefined,
      { fullErase },
    );
    setBoard(wizard, r.fc, r.ports, r.path);
    wizard.ui.flashed = true;
    phase.value = "done";
  } catch (e) {
    error.value = toUiError(e);
    phase.value = "error";
  }
}

async function onAction(id: GuidanceActionId): Promise<void> {
  if (id === "installDriver") {
    try {
      await api.system.installDfuDriver();
    } catch (e) {
      error.value = toUiError(e);
      return;
    }
  }
  if (id === "retryFullErase") return update(true);
  if (id === "retry" && !releases.value.length) return loadReleases();
  return update();
}

onMounted(loadReleases);
</script>

<template>
  <div v-if="fc">
    <h2>{{ t("steps.firmware.title") }}</h2>
    <p class="lead">{{ t("steps.firmware.description") }}</p>

    <dl class="kv">
      <dt>{{ t("firmware.current") }}</dt>
      <dd><bdi>{{ fc.fcVariant }} {{ fc.firmwareVersion }}</bdi></dd>
      <dt>{{ t("firmware.latest") }}</dt>
      <dd><bdi>{{ latest ?? t("firmware.unknown") }}</bdi></dd>
    </dl>

    <div v-if="!supported" class="callout callout--warn">
      <h3>{{ t("firmware.unsupportedTitle") }}</h3>
      <p>{{ t("firmware.unsupportedBody", { min: MIN_SUPPORTED_FIRMWARE }) }}</p>
    </div>
    <div v-else-if="upToDate && phase !== 'done'" class="callout callout--ok">{{ t("firmware.upToDate") }}</div>

    <template v-if="phase === 'idle' || phase === 'error'">
      <div v-if="releases.length" class="field">
        <label for="release">{{ t("firmware.release") }}</label>
        <select id="release" v-model="chosen">
          <option v-for="r in visibleReleases" :key="r.release" :value="r.release">
            {{ r.release }}{{ r.prerelease ? ` (${t("firmware.prerelease")})` : "" }}
          </option>
        </select>
        <label class="choice"><input v-model="showPre" type="checkbox" /> {{ t("firmware.showPrerelease") }}</label>
      </div>
      <div v-else-if="releasesError" class="callout callout--warn">
        <p>{{ isNotImplemented(releasesError) ? t("firmware.releasesUnavailable") : t("firmware.releasesFailed") }}</p>
      </div>

      <div class="btn-row">
        <button type="button" class="btn btn--primary" :disabled="!chosen" data-test="update" @click="update()">
          {{ t("firmware.updateTo", { version: chosen || "?" }) }}
        </button>
        <button v-if="supported" type="button" class="btn" :aria-pressed="wizard.ui.firmwareDecision === 'keep'" data-test="keep" @click="keep">
          {{ t("firmware.keep") }}
        </button>
      </div>
      <p class="hint">{{ t("firmware.backupNote") }}</p>
    </template>

    <template v-if="phase !== 'idle'">
      <div class="callout callout--warn" v-if="phase === 'running'">{{ t("firmware.doNotUnplug") }}</div>
      <StageList :labels="UPDATE_STAGES.map((s) => `firmware.stages.${s}`)" :current="stageIndex" :failed="phase === 'error'" :fraction="stage === 'flash' ? progress?.fraction : null" />
      <p v-if="stage === 'flash' && progress" class="hint">{{ t(`firmware.flashStage.${progress.stage}`) }}</p>
      <div v-if="phase === 'done'" class="callout callout--ok">{{ t("firmware.done", { version: fc.firmwareVersion }) }}</div>
    </template>

    <!-- Read protection: never removed without the explicit confirmation in this panel. -->
    <ReadProtectedPanel v-if="error && readProtected" :dfu-serial="dfuSerial" :has-backup="backupThisRun" @unprotected="update()" />
    <ErrorPanel v-else-if="error" :error="error" :busy="phase === 'running'" @action="onAction" />
  </div>
</template>
