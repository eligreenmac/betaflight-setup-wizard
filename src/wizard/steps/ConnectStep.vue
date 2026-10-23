<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// U2: port list, auto-detect, DFU / no-device states, driver help.
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api } from "../../api/commands";
import { demoState, forceDemo } from "../../api/fallback";
import DriverReplaceConfirm from "../../components/DriverReplaceConfirm.vue";
import ErrorPanel from "../../components/ErrorPanel.vue";
import { toUiError, uiError, type UiError } from "../../errors/codes";
import type { GuidanceActionId } from "../../errors/guidance";
import type { DfuDevice, HostPort, UsbDriverInfo } from "../../types/model";
import { installDfuDriverSafely } from "../dfuDriver";
import { probeOnboardReceiver } from "../receiver";
import { setBoard, wizard } from "../state";

defineProps<{ stepId: string }>();
const { t } = useI18n();

const hostPorts = ref<HostPort[]>([]);
const dfu = ref<DfuDevice[]>([]);
const selected = ref("");
const busy = ref(false);
const error = ref<UiError | null>(null);
const fc = computed(() => wizard.fc);

async function scan(): Promise<void> {
  error.value = null;
  try {
    hostPorts.value = await api.device.listPorts();
  } catch (e) {
    error.value = toUiError(e);
  }
  try {
    dfu.value = await api.flasher.listDfuDevices();
  } catch {
    dfu.value = []; // flasher not available yet: just don't show the DFU hint
  }
}

function autoPick(): string | null {
  if (selected.value) return selected.value;
  const fcs = hostPorts.value.filter((p) => p.kind === "FC_VCP");
  return (fcs[0] ?? (hostPorts.value.length === 1 ? hostPorts.value[0] : undefined))?.path ?? null;
}

async function connect(): Promise<void> {
  busy.value = true;
  error.value = null;
  try {
    await scan();
    if (error.value) return;
    const path = autoPick();
    if (!path) throw uiError("BOARD_NOT_FOUND", hostPorts.value.length ? "several serial ports, none looks like a flight controller" : "no serial ports found");
    await api.device.connect(path);
    const info = await probeOnboardReceiver(await api.device.identify());
    // Only Betaflight can report its UARTs; other firmware goes straight to "update".
    const ports = info.fcVariant === "BTFL" ? await api.config.readPorts() : [];
    setBoard(wizard, info, ports, path);
  } catch (e) {
    const ue = toUiError(e);
    // device_connect reboots a board left with unsaved changes; PENDING_UNSAVED_CHANGES here
    // means that reboot was not confirmed: the user has to power-cycle it (TODO(shared) code).
    error.value = ue.code === "PENDING_UNSAVED_CHANGES" ? uiError("REBOOT_NOT_CONFIRMED", `device_connect: ${ue.message}`) : ue;
  } finally {
    busy.value = false;
  }
}

async function disconnect(): Promise<void> {
  try {
    await api.device.disconnect();
  } catch {
    /* already gone */
  }
  wizard.fc = null;
  wizard.ports = [];
  wizard.connectedPath = null;
}

/** Devices whose current driver WinUSB would replace; set = waiting for confirmation. */
const pendingReplace = ref<UsbDriverInfo[] | null>(null);

/** Install the DFU driver; asks first when it would replace another driver. */
async function installDriver(confirmedReplace: boolean): Promise<boolean> {
  busy.value = true;
  try {
    const r = await installDfuDriverSafely(confirmedReplace);
    pendingReplace.value = r.installed ? null : r.replaces;
    return r.installed;
  } catch (e) {
    error.value = toUiError(e);
    pendingReplace.value = null;
    return false;
  } finally {
    busy.value = false;
  }
}

async function onAction(id: GuidanceActionId): Promise<void> {
  if (id === "installDriver" && !(await installDriver(false))) return;
  await connect();
}

async function confirmReplace(): Promise<void> {
  if (await installDriver(true)) await connect();
}

/** No board at hand: run every later call on the simulated board (banner stays on). */
async function useDemo(): Promise<void> {
  forceDemo();
  selected.value = "";
  await connect();
}

const label = (p: HostPort) => [p.path, p.product ?? p.manufacturer, p.kind === "FC_VCP" ? t("connect.looksLikeFc") : null].filter(Boolean).join(" · ");

onMounted(scan);
</script>

<template>
  <div>
    <h2>{{ t("steps.connect.title") }}</h2>
    <p class="lead">{{ t("steps.connect.description") }}</p>

    <template v-if="fc">
      <div class="callout callout--ok" data-test="connected">
        <h3>{{ t("connect.found", { board: fc.boardName }) }}</h3>
        <dl class="kv">
          <dt>{{ t("connect.board") }}</dt>
          <dd><bdi>{{ fc.boardName }}</bdi></dd>
          <dt>{{ t("connect.firmware") }}</dt>
          <dd><bdi>{{ fc.fcVariant }} {{ fc.firmwareVersion }}</bdi></dd>
          <dt v-if="fc.mcu">{{ t("connect.mcu") }}</dt>
          <dd v-if="fc.mcu"><bdi>{{ fc.mcu }}</bdi></dd>
          <dt>{{ t("connect.port") }}</dt>
          <dd><bdi>{{ wizard.connectedPath }}</bdi></dd>
        </dl>
      </div>
      <div class="btn-row">
        <button type="button" class="btn" @click="disconnect">{{ t("connect.disconnect") }}</button>
      </div>
    </template>

    <template v-else>
      <ol>
        <li>{{ t("connect.step1") }}</li>
        <li>{{ t("connect.step2") }}</li>
      </ol>
      <div v-if="hostPorts.length > 1" class="field">
        <label for="host-port">{{ t("connect.choosePort") }}</label>
        <select id="host-port" v-model="selected">
          <option value="">{{ t("connect.auto") }}</option>
          <option v-for="p in hostPorts" :key="p.path" :value="p.path">{{ label(p) }}</option>
        </select>
      </div>
      <p v-else-if="hostPorts.length === 0 && !error" class="hint">{{ t("connect.noPorts") }}</p>
      <div v-if="dfu.length" class="callout callout--info">
        <h3>{{ t("connect.dfuTitle") }}</h3>
        <p>{{ t("connect.dfuBody") }}</p>
      </div>
      <div class="btn-row">
        <button type="button" class="btn btn--primary" :disabled="busy" data-test="connect" @click="connect">
          {{ busy ? t("connect.searching") : t("connect.connect") }}
        </button>
        <button type="button" class="btn" :disabled="busy" @click="scan">{{ t("connect.rescan") }}</button>
      </div>
      <DriverReplaceConfirm v-if="pendingReplace" :replaces="pendingReplace" :busy="busy" @confirm="confirmReplace" @cancel="pendingReplace = null" />
      <ErrorPanel v-else-if="error" :error="error" :busy="busy" :hide="['goConnect']" @action="onAction" />
      <p v-if="!demoState.forced" class="hint">
        {{ t("demo.tryHint") }}
        <button type="button" class="btn btn--small btn--link" :disabled="busy" data-test="use-demo" @click="useDemo">{{ t("demo.try") }}</button>
      </p>
      <details>
        <summary>{{ t("connect.driverHelpTitle") }}</summary>
        <p>{{ t("connect.driverHelp1") }}</p>
        <p>{{ t("connect.driverHelp2") }}</p>
      </details>
    </template>
  </div>
</template>
