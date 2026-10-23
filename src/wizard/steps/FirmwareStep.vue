<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U3: release picker for the connected board's target, progress on flasher://progress.
// TODO(shared): target search filtered by MCU needs flasher_list_targets (stub today);
// the wizard uses the target the board reports, which covers the normal case.
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { api } from "../../api/commands";
import DriverReplaceConfirm from "../../components/DriverReplaceConfirm.vue";
import ErrorPanel from "../../components/ErrorPanel.vue";
import { installDfuDriverSafely } from "../dfuDriver";
import type { UsbDriverInfo } from "../../types/model";
import ReadProtectedPanel from "../../components/ReadProtectedPanel.vue";
import StageList from "../../components/StageList.vue";
import { isNotImplemented, toUiError, type UiError } from "../../errors/codes";
import type { GuidanceActionId } from "../../errors/guidance";
import type { FirmwareRelease, FlashProgress } from "../../types/model";
import { runFirmwareUpdate, UPDATE_STAGES, type UpdateStage } from "../flash";
import { goTo, steps } from "../nav";
import { optionLabel } from "../optionNames";
import { rxProtocolLabel } from "../receiver";
import { finishReflash, requirementsFor } from "../reflash";
import { setBoard, wizard } from "../state";
import { compareVersions, isSupportedFirmware, MIN_SUPPORTED_FIRMWARE } from "../version";
import { checkFirmwareLines, checkFirmwareSupport, lineSupport } from "../firmwareSupport";

defineProps<{ stepId: string }>();
const { t, te } = useI18n();

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
/** Options the board's old firmware had that this release doesn't (shown after a flash). */
const dropped = ref<string[]>([]);
/** Mid-wizard update ("Update the firmware to add this"), from the OSD or Receiver step. */
const reflash = computed(() => wizard.ui.reflash);
/** Shown after a mid-wizard update that left options out: go back by hand. */
const returnTo = ref<string | null>(null);
const readProtected = computed(() => error.value?.code === "READ_PROTECTED");

const fc = computed(() => wizard.fc);
/** Below the oldest version the wizard knows (or not Betaflight at all). */
const tooOld = computed(() => !!fc.value && !isSupportedFirmware(fc.value.fcVariant, fc.value.firmwareVersion));
/** The config engine has no support for this line (e.g. 2026.12 on a backend that stops at 2026.6). */
const notYet = computed(() => !!fc.value && !tooOld.value && lineSupport(wizard, fc.value.firmwareVersion) === "unsupported");
const supported = computed(() => !!fc.value && !tooOld.value && !notYet.value);
/** Releases the wizard can set up afterwards (unknown counts as usable; only a definite "no" excludes). */
const usable = (r: FirmwareRelease) => lineSupport(wizard, r.release) !== "unsupported";
const visibleReleases = computed(() => releases.value.filter((r) => showPre.value || !r.prerelease));
const anyUnusable = computed(() => visibleReleases.value.some((r) => !usable(r)));
/** Installing an older version than the board has (e.g. leaving an unsupported newer line). */
const isOlder = computed(() => !!fc.value && !!chosen.value && fc.value.fcVariant === "BTFL" && compareVersions(chosen.value, fc.value.firmwareVersion) < 0);
const latest = computed(() => wizard.ui.latestRelease);
const upToDate = computed(() => !!fc.value && !!latest.value && fc.value.fcVariant === "BTFL" && compareVersions(fc.value.firmwareVersion, latest.value) >= 0);
const stageIndex = computed(() => (phase.value === "done" ? UPDATE_STAGES.length : stage.value ? UPDATE_STAGES.indexOf(stage.value) : -1));

async function loadReleases(): Promise<void> {
  if (!fc.value) return;
  releasesError.value = null;
  try {
    const list = await api.flasher.listReleases(fc.value.boardName);
    const sorted = [...list].sort((a, b) => compareVersions(b.release, a.release));
    // Ask the config engine which of these lines it can set up (one call per line).
    await checkFirmwareLines(wizard, sorted.map((r) => r.release));
    releases.value = sorted;
    const stable = sorted.find((r) => !r.prerelease && usable(r));
    wizard.ui.latestRelease = stable?.release ?? null;
    chosen.value = stable?.release ?? sorted.find(usable)?.release ?? "";
  } catch (e) {
    releasesError.value = toUiError(e);
  }
}

function cancelReflash(): void {
  const back = finishReflash(wizard, false);
  if (back) goTo(back);
}

function stepTitle(id: string): string {
  return t(steps.value.find((s) => s.id === id)?.titleKey ?? id);
}

function keep(): void {
  wizard.ui.firmwareDecision = "keep";
}

async function update(fullErase = false, withRequirements = true): Promise<void> {
  if (!fc.value || !chosen.value) return;
  if (lineSupport(wizard, chosen.value) === "unsupported") return;
  const requirements = withRequirements ? requirementsFor(wizard) : null;
  phase.value = "running";
  error.value = null;
  dropped.value = [];
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
      // The receiver/OSD answers so far (+ what a mid-wizard update adds): the backend
      // turns them into build options (flasher_required_options) before anything is touched.
      { fullErase, requirements },
    );
    setBoard(wizard, r.fc, r.ports, r.path);
    wizard.ui.flashed = true;
    dropped.value = r.droppedOptions;
    phase.value = "done";
    if (wizard.ui.reflash) {
      const back = finishReflash(wizard);
      // Nothing left out: straight back to the step the user came from, answers intact.
      // Otherwise stay so the list above is read first.
      if (back && (dropped.value.length || !goTo(back))) returnTo.value = back;
    }
  } catch (e) {
    // A backend without flasher_required_options (before Device #74): a normal update keeps
    // the board's own options. Only before anything was touched, never for a mid-wizard update
    // (that one exists to add an option).
    if (requirements && !wizard.ui.reflash && stage.value === "download" && isNotImplemented(e)) return update(fullErase, false);
    error.value = toUiError(e);
    phase.value = "error";
  }
}

/** Devices whose current driver WinUSB would replace; set = waiting for confirmation. */
const pendingReplace = ref<UsbDriverInfo[] | null>(null);
const installing = ref(false);

async function installDriver(confirmedReplace: boolean): Promise<boolean> {
  installing.value = true;
  try {
    const r = await installDfuDriverSafely(confirmedReplace);
    pendingReplace.value = r.installed ? null : r.replaces;
    return r.installed;
  } catch (e) {
    error.value = toUiError(e);
    pendingReplace.value = null;
    return false;
  } finally {
    installing.value = false;
  }
}

async function confirmReplace(): Promise<void> {
  if (await installDriver(true)) await update();
}

async function onAction(id: GuidanceActionId): Promise<void> {
  if (id === "installDriver" && !(await installDriver(false))) return;
  if (id === "retryFullErase") return update(true);
  if (id === "retry" && !releases.value.length) return loadReleases();
  if (id === "pickRelease") {
    // REQUIRED_OPTION_UNSUPPORTED: nothing was touched; choose another release.
    error.value = null;
    phase.value = "idle";
    if (releases.value.some((r) => r.release === chosen.value && r.prerelease)) showPre.value = true;
    document.getElementById("release")?.focus();
    return;
  }
  return update();
}

onMounted(loadReleases);
// The board's own line (again after a flash or a reconnect).
watch(
  () => fc.value?.firmwareVersion,
  (v) => {
    if (v && fc.value?.fcVariant === "BTFL") void checkFirmwareSupport(wizard, v);
  },
  { immediate: true },
);
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

    <div v-if="tooOld" class="callout callout--warn">
      <h3>{{ t("firmware.unsupportedTitle") }}</h3>
      <p>{{ t("firmware.unsupportedBody", { min: MIN_SUPPORTED_FIRMWARE }) }}</p>
    </div>
    <div v-else-if="notYet" class="callout callout--warn" data-test="fw-not-supported-yet">
      <h3>{{ t("firmware.notSupportedYetTitle") }}</h3>
      <p>{{ t("firmware.notSupportedYetBody", { version: fc.firmwareVersion }) }}</p>
    </div>
    <div v-else-if="upToDate && phase !== 'done'" class="callout callout--ok">{{ t("firmware.upToDate") }}</div>

    <div v-if="reflash" class="callout callout--info" data-test="reflash-banner">
      <p>{{ t("reflash.banner", { step: stepTitle(reflash.returnTo) }) }}</p>
      <ul>
        <li v-if="reflash.requirements.osd?.videoSystem === 'HD'">{{ t("reflash.addsHd") }}</li>
        <li v-if="reflash.requirements.receiver?.protocol">{{ t("reflash.addsReceiver", { protocol: rxProtocolLabel(reflash.requirements.receiver.protocol, t, te) }) }}</li>
      </ul>
      <button v-if="phase !== 'running'" type="button" class="btn btn--small" data-test="reflash-abort" @click="cancelReflash">{{ t("reflash.cancelUpdate") }}</button>
    </div>

    <template v-if="phase === 'idle' || phase === 'error'">
      <div v-if="releases.length" class="field">
        <label for="release">{{ t("firmware.release") }}</label>
        <select id="release" v-model="chosen">
          <option v-for="r in visibleReleases" :key="r.release" :value="r.release" :disabled="!usable(r)">
            {{ r.release }}{{ r.prerelease ? ` (${t("firmware.prerelease")})` : "" }}{{ usable(r) ? "" : ` (${t("firmware.lineNotSupported")})` }}
          </option>
        </select>
        <p v-if="anyUnusable" class="hint" data-test="fw-some-not-supported">{{ t("firmware.someNotSupported") }}</p>
        <label class="choice"><input v-model="showPre" type="checkbox" /> {{ t("firmware.showPrerelease") }}</label>
      </div>
      <div v-else-if="releasesError" class="callout callout--warn">
        <p>{{ isNotImplemented(releasesError) ? t("firmware.releasesUnavailable") : t("firmware.releasesFailed") }}</p>
      </div>

      <div class="btn-row">
        <button type="button" class="btn btn--primary" :disabled="!chosen" data-test="update" @click="update()">
          {{ t(isOlder ? "firmware.installVersion" : "firmware.updateTo", { version: chosen || "?" }) }}
        </button>
        <button v-if="supported && !reflash" type="button" class="btn" :aria-pressed="wizard.ui.firmwareDecision === 'keep'" data-test="keep" @click="keep">
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
      <div v-if="phase === 'done' && dropped.length" class="callout callout--warn" data-test="fw-dropped">
        <p>{{ t("firmware.droppedIntro") }}</p>
        <ul>
          <li v-for="o in dropped" :key="o"><bdi>{{ optionLabel(o, t, te) }}</bdi></li>
        </ul>
        <p>{{ t("firmware.droppedAfter") }}</p>
      </div>
      <div v-if="phase === 'done' && returnTo" class="btn-row">
        <button type="button" class="btn btn--primary" data-test="reflash-return" @click="goTo(returnTo)">{{ t("reflash.backTo", { step: stepTitle(returnTo) }) }}</button>
      </div>
    </template>

    <!-- Read protection: never removed without the explicit confirmation in this panel. -->
    <ReadProtectedPanel v-if="error && readProtected" :dfu-serial="dfuSerial" :has-backup="backupThisRun" @unprotected="update()" />
    <DriverReplaceConfirm v-else-if="error && pendingReplace" :replaces="pendingReplace" :busy="installing" @confirm="confirmReplace" @cancel="pendingReplace = null" />
    <ErrorPanel v-else-if="error" :error="error" :busy="phase === 'running' || installing" @action="onAction" />
  </div>
</template>
