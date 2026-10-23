// SPDX-License-Identifier: GPL-3.0-or-later
// Shared reactive wizard state (one instance per app). Steps read/write `model`.
// `model` is the firmware-independent answer set sent to config_generate / config_verify.
// `ui` holds answers and progress that are not part of the shared DroneModel contract.

import { reactive } from "vue";
import {
  emptyDroneModel,
  type ApplyResult,
  type Backup,
  type CliScript,
  type DroneModel,
  type FcInfo,
  type FcSerialPort,
  type ReceiverKind,
  type RxProtocol,
  type VtxProtocol,
  type VtxKind,
  type VerifyReport,
} from "../types/model";
import { clearUnneededRolePorts, prefillFromBoard, type RoleOptions } from "./ports";
import { MAX_AUX, MODES, modeKeyOf, modeName, normalizeModeNames, SWITCH_POSITIONS, type ModeKey } from "./switches";
import type { Airmode } from "./modelExt";
import { defaultTelemetry } from "./receiver";
import { emptyExt, type ModelExt } from "./modelExt";
import type { SensorsInfo } from "../api/pending";

export type FirmwareDecision = "keep" | "update" | null;
/** CRSF on a UART can be ExpressLRS or TBS Crossfire; only ExpressLRS has a binding phrase. */
export type CrsfFamily = "ELRS" | "CROSSFIRE" | null;
/** ExpressLRS binding: "phrase" = binding phrase (SPI: hashed to receiver.elrsUid); "button" = bind mode. */
export type ElrsBinding = "phrase" | "button" | null;
/** PREARM on the Switches step: added, explicitly skipped, or not decided (optional). */
export type PrearmChoice = "add" | "skip" | null;
/** Beepers step: keep the board's beeps, or choose them (starts from the recommendation). */
export type BeeperMode = "keep" | "choose" | null;
export type CalibrationStatus = "idle" | "running" | "done" | "failed";
export type ApplyStatus = "idle" | "generating" | "applying" | "verifying" | "verified" | "mismatch" | "failed";

export interface UiState extends RoleOptions {
  /** Latest stable release for this board (null = unknown / offline). */
  latestRelease: string | null;
  firmwareDecision: FirmwareDecision;
  /** Firmware was flashed in this session. */
  flashed: boolean;
  /** Mandatory "all propellers removed" acknowledgement before any motor test. */
  propsOffAck: boolean;
  /** Mandatory "battery disconnected, USB power only" acknowledgement (no firmware motor timeout). */
  batteryOffAck: boolean;
  motorOrderOk: boolean;
  motorDirectionOk: boolean;
  /** null = not probed yet; false = backend has no device_motor_test_* commands yet. */
  motorTestAvailable: boolean | null;
  /** Script from config_plan for the current model (Review); exactly what Apply sends. */
  script: CliScript | null;
  /** The backend has no config_plan yet: Review shows a preview and Apply is blocked. */
  planUnavailable: boolean;
  applyStatus: ApplyStatus;
  applyResult: ApplyResult | null;
  verify: VerifyReport | null;
  /** Receiver protocol CRSF: which radio system (asked on the Binding step). */
  crsfFamily: CrsfFamily;
  /** ExpressLRS binding choice (Binding step). The phrase itself is never stored. */
  elrsBinding: ElrsBinding;
  /** Backups created in this session (fallback list when config_list_backups is missing). */
  sessionBackups: Backup[];
  prearmChoice: PrearmChoice;
  beeperMode: BeeperMode;
  /** Sensors reported by device_sensors (null = not read yet / unavailable). */
  sensors: SensorsInfo | null;
  accCalibration: CalibrationStatus;
  /** Orientation check answer: true = the picture moved the same way. */
  orientationOk: boolean | null;
}

export interface WizardState {
  model: DroneModel;
  fc: FcInfo | null;
  ports: FcSerialPort[];
  connectedPath: string | null;
  ui: UiState;
  /** Proposed DroneModel fields (wizard/modelExt.ts); null = keep the board's value. */
  ext: ModelExt;
}

function emptyUi(): UiState {
  return {
    escTelemetry: false,
    latestRelease: null,
    firmwareDecision: null,
    flashed: false,
    propsOffAck: false,
    batteryOffAck: false,
    motorOrderOk: false,
    motorDirectionOk: false,
    motorTestAvailable: null,
    script: null,
    planUnavailable: false,
    applyStatus: "idle",
    applyResult: null,
    verify: null,
    sessionBackups: [],
    crsfFamily: null,
    elrsBinding: null,
    prearmChoice: null,
    beeperMode: null,
    sensors: null,
    accCalibration: "idle",
    orientationOk: null,
  };
}

/** Wizard defaults on top of the shared empty model: an arm switch on AUX1 high. */
export function initialModel(): DroneModel {
  const m = emptyDroneModel();
  m.switches = [{ mode: MODES.arm, auxChannel: 0, rangeMin: SWITCH_POSITIONS.high[0], rangeMax: SWITCH_POSITIONS.high[1] }];
  return m;
}

export function createWizardState(): WizardState {
  return { model: initialModel(), fc: null, ports: [], connectedPath: null, ui: emptyUi(), ext: emptyExt() };
}

export const wizard = reactive<WizardState>(createWizardState());

/** Reset everything (tests, "start over"). */
export function resetWizard(s: WizardState = wizard): void {
  Object.assign(s, createWizardState());
}

/** Any answer changed after Review/Apply: the generated script and verification are stale. */
export function markDirty(s: WizardState = wizard): void {
  s.ui.script = null;
  s.ui.applyResult = null;
  s.ui.verify = null;
  s.ui.applyStatus = "idle";
}

/** A (possibly different) board was identified: copy identity and refresh the port list. */
export function setBoard(s: WizardState, fc: FcInfo, ports: FcSerialPort[], path: string | null): void {
  const changed = s.fc !== null && s.fc.boardName !== fc.boardName;
  s.fc = fc;
  s.ports = ports;
  s.connectedPath = path;
  s.model.fc = { boardName: fc.boardName, firmwareVersion: fc.firmwareVersion, apiVersion: fc.apiVersion };
  normalizeModeNames(s.model.switches);
  if (changed) {
    // Port names from another board mean nothing here.
    s.model.receiver.port = null;
    s.model.vtx.port = null;
    s.model.gps.port = null;
    s.model.esc.telemetryPort = null;
    s.ui.motorOrderOk = false;
    s.ui.motorDirectionOk = false;
    // Sensors, calibration and mounting belong to the other board.
    s.ui.sensors = null;
    s.ui.accCalibration = "idle";
    s.ui.orientationOk = null;
    s.ext.sensors = null;
    s.ext.boardAlignment = null;
  }
  prefillFromBoard(s.ports, s.model, s.ui);
  markDirty(s);
}

export function setReceiverKind(s: WizardState, kind: ReceiverKind): void {
  if (s.model.receiver.kind !== kind) {
    s.model.receiver.kind = kind;
    s.model.receiver.protocol = kind === "SERIAL" ? "CRSF" : kind === "SPI" ? "EXPRESSLRS" : null;
    // No receiver: keep the shared model default (telemetry true); the engine ignores it.
    s.model.receiver.telemetry = s.model.receiver.protocol ? defaultTelemetry(s.model.receiver.protocol) : true;
    resetBinding(s);
  }
  afterComponentsChange(s);
}

export function setReceiverProtocol(s: WizardState, protocol: RxProtocol): void {
  if (s.model.receiver.protocol !== protocol) {
    s.model.receiver.protocol = protocol;
    s.model.receiver.telemetry = defaultTelemetry(protocol);
    resetBinding(s);
  }
  markDirty(s);
}

/** A different receiver: forget the binding answers and any UID derived from a phrase. */
function resetBinding(s: WizardState): void {
  s.ui.crsfFamily = null;
  s.ui.elrsBinding = null;
  s.model.receiver.elrsUid = null;
}

export function setVtxProtocol(s: WizardState, protocol: VtxProtocol): void {
  s.model.vtx.protocol = protocol;
  // A different protocol needs different functions: re-run prefill for the new mask.
  afterComponentsChange(s);
}

export function setVtxKind(s: WizardState, kind: VtxKind): void {
  if (s.model.vtx.kind !== kind) {
    s.model.vtx.kind = kind;
    s.model.vtx.protocol = kind === "DIGITAL" ? "MSP" : kind === "ANALOG" ? "SMARTAUDIO" : null;
    s.model.vtx.digitalSystem = kind === "DIGITAL" ? "DJI" : null;
  }
  afterComponentsChange(s);
}

export function setGpsEnabled(s: WizardState, enabled: boolean): void {
  s.model.gps.enabled = enabled;
  s.model.gps.provider = enabled ? (s.model.gps.provider ?? "UBLOX") : null;
  afterComponentsChange(s);
}

export function setEscTelemetry(s: WizardState, enabled: boolean): void {
  s.ui.escTelemetry = enabled;
  afterComponentsChange(s);
}

function afterComponentsChange(s: WizardState): void {
  clearUnneededRolePorts(s.model, s.ui);
  prefillFromBoard(s.ports, s.model, s.ui);
  markDirty(s);
}

/** Add a mode switch on the first free AUX channel (high position). No-op if already there. */
export function addModeSwitch(s: WizardState, k: ModeKey): void {
  if (s.model.switches.some((x) => modeKeyOf(x.mode) === k)) return;
  const auxUsed = new Set(s.model.switches.map((x) => x.auxChannel));
  let aux = 0;
  while (auxUsed.has(aux) && aux < MAX_AUX - 1) aux++;
  s.model.switches.push({ mode: modeName(k), auxChannel: aux, rangeMin: SWITCH_POSITIONS.high[0], rangeMax: SWITCH_POSITIONS.high[1] });
  if (k === "prearm") s.ui.prearmChoice = "add";
  if (k === "airMode") s.ext.airmode = "switch";
  markDirty(s);
}

export function removeModeSwitch(s: WizardState, index: number): void {
  const k = modeKeyOf(s.model.switches[index]?.mode ?? "");
  if (k === "arm") return;
  s.model.switches.splice(index, 1);
  if (k === "prearm") s.ui.prearmChoice = null;
  // Without its switch, "air mode on a switch" is no answer any more: back to "keep".
  if (k === "airMode" && s.ext.airmode === "switch") s.ext.airmode = null;
  markDirty(s);
}

/** PREARM: add it on a switch, or explicitly skip it (removes any PREARM switch). */
export function setPrearm(s: WizardState, choice: "add" | "skip"): void {
  if (choice === "add") addModeSwitch(s, "prearm");
  else {
    s.model.switches = s.model.switches.filter((x) => modeKeyOf(x.mode) !== "prearm");
    s.ui.prearmChoice = "skip";
    markDirty(s);
  }
}

/** AIRMODE: "always" (feature), "switch" (AIR MODE box on a switch), "off", or null = keep. */
export function setAirmode(s: WizardState, mode: Airmode | null): void {
  s.ext.airmode = mode;
  if (mode === "switch") addModeSwitch(s, "airMode");
  else s.model.switches = s.model.switches.filter((x) => modeKeyOf(x.mode) !== "airMode");
  markDirty(s);
}
