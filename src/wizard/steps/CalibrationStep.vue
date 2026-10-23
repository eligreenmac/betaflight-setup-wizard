<script setup lang="ts">
// SPDX-License-Identifier: GPL-3.0-or-later
// Sensors, accelerometer calibration and the "which way is the board mounted?" check.
// Uses the Device #63 commands (via the api/pending.ts adapter). There is no live
// board-alignment command: the preview rotates the reported attitude here, and the
// chosen alignment is written by the config script (ext.boardAlignment, null = keep).
// The attitude stream reports with the FC's CURRENT alignment applied (#63 §4.3); the
// preview undoes it using device_sensors `alignment` (0/0/0 if not reported).
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api } from "../../api/commands";
import { onEvent, type Unlisten } from "../../api/events";
import { sourceMismatch } from "../../api/fallback";
import { ATTITUDE_START_COMMAND, attitudeToEuler, PENDING_EVENTS, pendingApi, SENSORS_COMMAND, type Attitude, type AttitudeStopped } from "../../api/pending";
import ErrorPanel from "../../components/ErrorPanel.vue";
import { toUiError, type UiError } from "../../errors/codes";
import { accModeSwitches, calibrationWarnings, mayHaveUnsavedChanges, sensorIssues } from "../guards";
import type { BoardAlignment } from "../modelExt";
import { motorTest } from "../motorTest";
import { ALIGNMENT_PRESETS, alignedAttitude, sameAlignment, tiltDirection, type Euler } from "../orientation";
import { markDirty, wizard } from "../state";
import { modeKeyOf } from "../switches";
import { ACC_MODES } from "../switches";

defineProps<{ stepId: string }>();
const { t } = useI18n();

// ---- sensors
const sensorsState = ref<"loading" | "ok" | "unavailable" | "error">("loading");
const sensorsError = ref<UiError | null>(null);
const sensors = computed(() => wizard.ui.sensors);
type SensorKey = "acc" | "baro" | "mag";
/** Planned state of a sensor: the user's answer, else what the FC has now (null = keep). */
const isOn = (k: SensorKey): boolean => wizard.ext.sensors?.[k] ?? sensors.value?.[k].enabled ?? false;
const accOn = computed(() => isOn("acc"));
const sensorsSimulated = computed(() => sourceMismatch(SENSORS_COMMAND));
const warnings = computed(() => calibrationWarnings(wizard));
const errors = computed(() => sensorIssues(wizard));
const unsaved = computed(() => mayHaveUnsavedChanges(wizard));

/** Only the touched sensor gets a value; the others stay null (= keep). */
function setSensor(which: SensorKey, on: boolean): void {
  wizard.ext.sensors = { acc: null, baro: null, mag: null, ...wizard.ext.sensors, [which]: on };
  markDirty();
}
const anySensorAnswer = computed(() => !!wizard.ext.sensors && Object.values(wizard.ext.sensors).some((v) => v !== null));

// Fixes for "accelerometer off, but a mode that needs it is on a switch".
function fixAccOn(): void {
  // Back to the FC's own setting if it has the acc on, else switch it on (AUTO).
  setSensor("acc", true);
  if (wizard.ext.sensors && sensors.value?.acc.enabled) wizard.ext.sensors.acc = null;
  markDirty();
}
function fixRemoveModes(): void {
  wizard.model.switches = wizard.model.switches.filter((m) => !ACC_MODES.includes(modeKeyOf(m.mode)!));
  markDirty();
}

async function loadSensors(): Promise<void> {
  sensorsState.value = "loading";
  sensorsError.value = null;
  try {
    wizard.ui.sensors = await pendingApi.sensors();
    sensorsState.value = "ok";
  } catch (e) {
    const ue = toUiError(e);
    sensorsState.value = ue.code === "NOT_IMPLEMENTED" ? "unavailable" : "error";
    sensorsError.value = ue;
  }
}

// ---- accelerometer calibration
const calError = ref<UiError | null>(null);
async function calibrate(): Promise<void> {
  // Calibration saves to EEPROM together with any pending RAM change: never with unsaved changes.
  if (wizard.ui.accCalibration === "running" || unsaved.value) return;
  calError.value = null;
  wizard.ui.accCalibration = "running";
  const wasLive = live.value === "on";
  // The calibration needs the board to itself: pause the live view meanwhile.
  if (wasLive) await stopAttitude();
  try {
    await pendingApi.accCalibrate();
    wizard.ui.accCalibration = "done";
  } catch (e) {
    calError.value = toUiError(e);
    wizard.ui.accCalibration = "failed";
  }
  if (wasLive) await startAttitude();
}

// ---- live attitude
const live = ref<"off" | "starting" | "on" | "busy" | "unavailable" | "error">("off");
const liveError = ref<UiError | null>(null);
/** Why the live view stopped on its own (DISCONNECTED / ERROR), for the retry message. */
const stopReason = ref<string | null>(null);
const reported = ref<Euler | null>(null);
let offs: Unlisten[] = [];
const attitudeSimulated = computed(() => sourceMismatch(ATTITUDE_START_COMMAND));
// Keep (null) = the alignment the FC already has.
const current = computed<BoardAlignment>(() => wizard.ui.sensors?.alignment ?? { roll: 0, pitch: 0, yaw: 0 });
const preview = computed<BoardAlignment>(() => wizard.ext.boardAlignment ?? current.value);
// The FC applies its current alignment before reporting; undo it for the preview (0/0/0 if unknown).
const shown = computed(() => (reported.value ? alignedAttitude(reported.value, preview.value, current.value) : null));
const currentIsPreset = computed(() => ALIGNMENT_PRESETS.some((p) => sameAlignment(current.value, p.align)));
const direction = computed(() => (shown.value ? tiltDirection(shown.value) : null));

async function startAttitude(): Promise<void> {
  live.value = "starting";
  liveError.value = null;
  stopReason.value = null;
  // Only one stream at a time on the port: stop the RC stream and any motor test first.
  await api.device.rcStreamStop().catch(() => undefined);
  await motorTest.stop("navigation");
  try {
    await pendingApi.attitudeStreamStart(10);
    live.value = "on";
  } catch (e) {
    const ue = toUiError(e);
    liveError.value = ue;
    live.value = ue.code === "PORT_BUSY" ? "busy" : ue.code === "NOT_IMPLEMENTED" ? "unavailable" : "error";
  }
}
async function stopAttitude(): Promise<void> {
  if (live.value !== "on" && live.value !== "starting") return;
  live.value = "off";
  await pendingApi.attitudeStreamStop().catch(() => undefined);
}

function answer(ok: boolean): void {
  wizard.ui.orientationOk = ok;
}
function choose(align: BoardAlignment): void {
  wizard.ext.boardAlignment = { ...align };
  markDirty();
}
function keepAlignment(): void {
  wizard.ext.boardAlignment = null;
  markDirty();
}

/** Side view (nose to the right) and rear view, both in degrees for an SVG rotate. */
const sideAngle = computed(() => -(shown.value?.pitch ?? 0));
const rearAngle = computed(() => {
  const r = shown.value?.roll ?? 0;
  return ((r + 540) % 360) - 180;
});
const upsideDown = computed(() => Math.abs(rearAngle.value) > 90);

onMounted(async () => {
  if (!wizard.connectedPath) {
    sensorsState.value = "unavailable";
    live.value = "unavailable";
    return;
  }
  offs.push(await onEvent(PENDING_EVENTS.attitude, (a: Attitude) => (reported.value = attitudeToEuler(a))));
  offs.push(
    await onEvent(PENDING_EVENTS.attitudeStopped, (s: AttitudeStopped) => {
      if (s.reason === "REQUESTED" || live.value !== "on") return;
      reported.value = null;
      stopReason.value = s.reason;
      liveError.value = s.error ? toUiError(s.error) : null;
      live.value = "error";
    }),
  );
  await loadSensors();
  await startAttitude();
});
onBeforeUnmount(() => {
  offs.forEach((o) => o());
  offs = [];
  void stopAttitude();
});
</script>

<template>
  <div>
    <h2>{{ t("steps.calibration.title") }}</h2>
    <p class="lead">{{ t("steps.calibration.description") }}</p>

    <!-- Sensors -->
    <section class="subcard" data-section="sensors">
      <h3>{{ t("calibration.sensors.title") }}</h3>
      <p v-if="sensorsState === 'loading'" class="hint" role="status">{{ t("calibration.sensors.loading") }}</p>
      <div v-else-if="sensorsState === 'unavailable'" class="callout callout--warn" data-test="sensors-unavailable">{{ t("calibration.sensors.unavailable") }}</div>
      <template v-else-if="sensorsState === 'error' && sensorsError">
        <ErrorPanel :error="sensorsError" @action="loadSensors" />
        <button type="button" class="btn btn--small" @click="loadSensors">{{ t("actions.tryAgain") }}</button>
      </template>
      <template v-else-if="sensors">
        <p v-if="sensorsSimulated" class="callout callout--warn" data-test="sensors-simulated">{{ t("calibration.sensors.simulated") }}</p>
        <ul class="sensor-list">
          <li data-sensor="gyro">
            <strong>{{ t("calibration.sensors.gyro") }}</strong>
            <span class="badge">{{ sensors.gyro.present ? t("calibration.sensors.alwaysOn") : t("calibration.sensors.missing") }}</span>
            <small class="hint">{{ t("calibration.sensors.gyroHint") }}</small>
          </li>
          <li data-sensor="acc">
            <label v-if="sensors.acc.present || !sensors.acc.enabled" class="choice">
              <input type="checkbox" data-test="acc-toggle" :checked="accOn" @change="setSensor('acc', ($event.target as HTMLInputElement).checked)" />
              <strong>{{ t("calibration.sensors.acc") }}</strong>
            </label>
            <template v-else>
              <strong>{{ t("calibration.sensors.acc") }}</strong>
              <span class="badge">{{ t("calibration.sensors.missing") }}</span>
            </template>
            <small class="hint">{{ t("calibration.sensors.accHint") }}</small>
            <small v-if="!sensors.acc.present && !sensors.acc.enabled" class="hint" data-test="acc-off">{{ t("calibration.sensors.offHint") }}</small>
          </li>
          <li v-if="sensors.baro.present || !sensors.baro.enabled" data-sensor="baro">
            <label class="choice">
              <input type="checkbox" data-test="baro-toggle" :checked="isOn('baro')" @change="setSensor('baro', ($event.target as HTMLInputElement).checked)" />
              <strong>{{ t("calibration.sensors.baro") }}</strong>
            </label>
            <small class="hint">{{ t("calibration.sensors.baroHint") }}</small>
            <small v-if="!sensors.baro.present && !sensors.baro.enabled" class="hint">{{ t("calibration.sensors.offHint") }}</small>
          </li>
          <li v-if="sensors.mag.present" data-sensor="mag">
            <label class="choice">
              <input type="checkbox" data-test="mag-toggle" :checked="isOn('mag')" @change="setSensor('mag', ($event.target as HTMLInputElement).checked)" />
              <strong>{{ t("calibration.sensors.mag") }}</strong>
            </label>
            <small class="hint">{{ t("calibration.sensors.magHint") }}</small>
          </li>
        </ul>
        <p v-if="!anySensorAnswer" class="hint">{{ t("calibration.sensors.keepHint") }}</p>
      </template>
      <div v-for="e in errors" :key="e.key" class="callout callout--danger" role="alert" :data-error="e.key">
        <p>{{ t(e.key, e.params ?? {}) }}</p>
        <div class="btn-row">
          <button type="button" class="btn btn--small btn--primary" data-test="fix-acc-on" @click="fixAccOn">{{ t("calibration.sensors.fixAccOn") }}</button>
          <button type="button" class="btn btn--small" data-test="fix-remove-modes" @click="fixRemoveModes">
            {{ t("calibration.sensors.fixRemoveModes", { modes: accModeSwitches(wizard).join(", ") }) }}
          </button>
        </div>
      </div>
      <div v-for="w in warnings" :key="w.key" class="callout callout--warn" :data-warning="w.key">{{ t(w.key) }}</div>
    </section>

    <!-- Accelerometer calibration -->
    <section v-if="sensors?.acc.present" class="subcard" data-section="acc-calibration">
      <h3>{{ t("calibration.acc.title") }}</h3>
      <ol>
        <li>{{ t("calibration.acc.step1") }}</li>
        <li>{{ t("calibration.acc.step2") }}</li>
        <li>{{ t("calibration.acc.step3") }}</li>
      </ol>
      <div class="btn-row">
        <button
          type="button"
          class="btn btn--primary"
          data-test="calibrate"
          :disabled="wizard.ui.accCalibration === 'running' || !accOn || unsaved"
          @click="calibrate"
        >
          {{ wizard.ui.accCalibration === "failed" ? t("calibration.acc.retry") : t("calibration.acc.calibrate") }}
        </button>
      </div>
      <p class="hint" data-test="saves-now">{{ t("calibration.acc.savesNow") }}</p>
      <p v-if="!accOn" class="hint">{{ t("calibration.acc.accOff") }}</p>
      <p v-if="unsaved" class="callout callout--warn" data-test="unsaved-changes">{{ t("calibration.acc.unsaved") }}</p>
      <p v-if="sensors?.accCalibrated === false && wizard.ui.accCalibration !== 'done'" class="callout callout--warn" data-test="needs-calibration">
        {{ t("calibration.acc.needed") }}
      </p>
      <p v-if="wizard.ui.accCalibration === 'running'" class="callout busy-row" role="status" data-test="calibrating">
        <span class="spinner" aria-hidden="true" />
        {{ t("calibration.acc.running") }}
      </p>
      <p v-else-if="wizard.ui.accCalibration === 'done'" class="callout callout--ok" role="status" data-test="calibrated">{{ t("calibration.acc.done") }}</p>
      <template v-else-if="wizard.ui.accCalibration === 'failed' && calError">
        <ErrorPanel :error="calError" @action="calibrate" />
        <p class="hint" data-test="calibration-failed">{{ t("calibration.acc.failedHint") }}</p>
      </template>
      <p class="hint">{{ t("calibration.acc.optional") }}</p>
    </section>

    <!-- Orientation check -->
    <section class="subcard" data-section="orientation">
      <h3>{{ t("calibration.orientation.title") }}</h3>
      <div v-if="live === 'unavailable'" class="callout callout--warn" data-test="attitude-unavailable">{{ t("calibration.orientation.unavailable") }}</div>
      <template v-else-if="live === 'busy' || live === 'error'">
        <div class="callout callout--warn" data-test="attitude-busy">
          {{
            live === "busy"
              ? t("calibration.orientation.busy")
              : stopReason === "DISCONNECTED"
                ? t("calibration.orientation.stoppedDisconnected")
                : stopReason
                  ? t("calibration.orientation.stoppedError")
                  : t("calibration.orientation.failed")
          }}
        </div>
        <button type="button" class="btn btn--small" data-test="attitude-retry" @click="startAttitude">{{ t("actions.tryAgain") }}</button>
      </template>
      <template v-else>
        <p>{{ t("calibration.orientation.instructions") }}</p>
        <div class="attitude" dir="ltr" data-test="attitude-view" :data-direction="direction ?? ''">
          <figure>
            <svg viewBox="-60 -40 120 80" width="180" height="120" role="img" :aria-label="t('calibration.orientation.sideView')">
              <line x1="-58" y1="0" x2="58" y2="0" class="attitude__horizon" />
              <g :transform="`rotate(${sideAngle})`">
                <rect x="-40" y="-4" width="80" height="8" rx="3" class="attitude__frame" />
                <polygon points="40,-10 54,0 40,10" class="attitude__nose" />
                <rect x="-46" y="-12" width="14" height="4" class="attitude__prop" />
                <rect x="32" y="-12" width="14" height="4" class="attitude__prop" />
              </g>
            </svg>
            <figcaption>{{ t("calibration.orientation.sideView") }}</figcaption>
          </figure>
          <figure>
            <svg viewBox="-60 -40 120 80" width="180" height="120" role="img" :aria-label="t('calibration.orientation.rearView')">
              <line x1="-58" y1="0" x2="58" y2="0" class="attitude__horizon" />
              <g :transform="`rotate(${rearAngle})`">
                <rect x="-40" y="-4" width="80" height="8" rx="3" class="attitude__frame" />
                <rect x="-46" y="-12" width="14" height="4" class="attitude__prop" />
                <rect x="32" y="-12" width="14" height="4" class="attitude__prop" />
                <text x="44" y="22" class="attitude__label">R</text>
                <text x="-50" y="22" class="attitude__label">L</text>
              </g>
            </svg>
            <figcaption>{{ t("calibration.orientation.rearView") }}</figcaption>
          </figure>
        </div>
        <p class="hint" role="status">
          {{ live === "starting" || !direction ? t("calibration.orientation.waiting") : t(`calibration.orientation.direction.${direction}`) }}
          <span v-if="upsideDown"> {{ t("calibration.orientation.upsideDown") }}</span>
        </p>
        <p v-if="attitudeSimulated" class="callout callout--warn" data-test="attitude-simulated">{{ t("calibration.orientation.simulated") }}</p>

        <p><strong>{{ t("calibration.orientation.question") }}</strong></p>
        <div class="choice-row" role="radiogroup" :aria-label="t('calibration.orientation.question')">
          <label><input type="radio" name="orientation" data-orientation="yes" :checked="wizard.ui.orientationOk === true" @change="answer(true)" /> {{ t("calibration.orientation.yes") }}</label>
          <label><input type="radio" name="orientation" data-orientation="no" :checked="wizard.ui.orientationOk === false" @change="answer(false)" /> {{ t("calibration.orientation.no") }}</label>
        </div>

        <div v-if="wizard.ui.orientationOk === false || wizard.ext.boardAlignment" data-test="alignment-presets">
          <p>{{ t("calibration.orientation.presetsIntro") }}</p>
          <div class="preset-grid">
            <button
              v-for="p in ALIGNMENT_PRESETS"
              :key="p.id"
              type="button"
              class="btn btn--small"
              :data-preset="p.id"
              :aria-pressed="sameAlignment(preview, p.align)"
              :disabled="attitudeSimulated"
              @click="choose(p.align)"
            >
              {{ t(`calibration.orientation.presets.${p.id}`) }}
            </button>
          </div>
          <p class="hint" data-test="alignment-current">
            {{ t("calibration.orientation.current", { roll: current.roll, pitch: current.pitch, yaw: current.yaw }) }}
            <span v-if="!currentIsPreset"> {{ t("calibration.orientation.currentCustom") }}</span>
          </p>
          <p class="hint">{{ t("calibration.orientation.presetsHint") }}</p>
          <p class="hint" data-test="alignment-after-apply">{{ t("calibration.orientation.afterApply") }}</p>
          <button v-if="wizard.ext.boardAlignment" type="button" class="btn btn--small btn--link" data-test="alignment-keep" @click="keepAlignment">
            {{ t("calibration.orientation.keep") }}
          </button>
        </div>
      </template>
    </section>
  </div>
</template>
