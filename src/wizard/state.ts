// SPDX-License-Identifier: GPL-3.0-or-later
// Shared reactive wizard state (one instance per app). Steps read/write `model`.
// `model` is the firmware-independent answer set sent to config_generate / config_verify.
// `ui` holds answers and progress that are not part of the shared DroneModel contract.

import { reactive } from "vue";
import {
  emptyDroneModel,
  type ApplyResult,
  type CliScript,
  type DroneModel,
  type FcInfo,
  type FcSerialPort,
  type ReceiverKind,
  type VtxKind,
} from "../types/model";
import type { VerifyOutcome } from "../api/upcoming";
import { clearUnneededRolePorts, prefillFromBoard, type RoleOptions } from "./ports";
import { MODES, SWITCH_POSITIONS } from "./switches";

export type FirmwareDecision = "keep" | "update" | null;
export type ApplyStatus = "idle" | "generating" | "applying" | "verifying" | "verified" | "mismatch" | "failed";

export interface UiState extends RoleOptions {
  /** Latest stable release for this board (null = unknown / offline). */
  latestRelease: string | null;
  firmwareDecision: FirmwareDecision;
  /** Firmware was flashed in this session. */
  flashed: boolean;
  /** Mandatory "all propellers removed" acknowledgement before any motor test. */
  propsOffAck: boolean;
  motorOrderOk: boolean;
  motorDirectionOk: boolean;
  /** null = not probed yet; false = backend has no motor_test_* commands yet. */
  motorTestAvailable: boolean | null;
  /** Script from config_generate for the current model (Review). */
  script: CliScript | null;
  applyStatus: ApplyStatus;
  applyResult: ApplyResult | null;
  verify: VerifyOutcome | null;
}

export interface WizardState {
  model: DroneModel;
  fc: FcInfo | null;
  ports: FcSerialPort[];
  connectedPath: string | null;
  ui: UiState;
}

function emptyUi(): UiState {
  return {
    escTelemetry: false,
    latestRelease: null,
    firmwareDecision: null,
    flashed: false,
    propsOffAck: false,
    motorOrderOk: false,
    motorDirectionOk: false,
    motorTestAvailable: null,
    script: null,
    applyStatus: "idle",
    applyResult: null,
    verify: null,
  };
}

/** Wizard defaults on top of the shared empty model: an arm switch on AUX1 high. */
export function initialModel(): DroneModel {
  const m = emptyDroneModel();
  m.switches = [{ mode: MODES.arm, auxChannel: 0, rangeMin: SWITCH_POSITIONS.high[0], rangeMax: SWITCH_POSITIONS.high[1] }];
  return m;
}

export function createWizardState(): WizardState {
  return { model: initialModel(), fc: null, ports: [], connectedPath: null, ui: emptyUi() };
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
  if (changed) {
    // Port names from another board mean nothing here.
    s.model.receiver.port = null;
    s.model.vtx.port = null;
    s.model.gps.port = null;
    s.model.esc.telemetryPort = null;
    s.ui.motorOrderOk = false;
    s.ui.motorDirectionOk = false;
  }
  prefillFromBoard(s.ports, s.model, s.ui);
  markDirty(s);
}

export function setReceiverKind(s: WizardState, kind: ReceiverKind): void {
  if (s.model.receiver.kind !== kind) {
    s.model.receiver.kind = kind;
    s.model.receiver.protocol = kind === "SERIAL" ? "CRSF" : kind === "SPI" ? "EXPRESSLRS" : null;
    s.model.receiver.telemetry = kind === "SERIAL" || kind === "SPI";
  }
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
