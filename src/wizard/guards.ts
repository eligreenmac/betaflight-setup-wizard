// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Per-step guards and validators, keyed by the step ids in steps.json.
 * steps.json decides WHICH steps exist and when they are visible (engine.ts);
 * this map decides WHEN a step may be entered and what blocks "Next".
 * Pure functions over WizardState, unit tested in tests/guards.test.ts.
 */
import type { StepDef } from "./engine";
import { exclusionReason, getRolePort, neededRoles, type PortRole } from "./ports";
import type { WizardState } from "./state";
import { effectiveSystem, hdSupport } from "./osd";
import { ACC_MODES, BARO_MODES, MAX_AUX, MODE_MIN_FIRMWARE, MODES, modeAvailable, modeKeyOf, NEEDS_DSHOT, RANGE_MAX, RANGE_MIN, RANGE_STEP } from "./switches";
import { catalogState } from "../api/catalog";
import { validAlignment } from "./orientation";
import { bindingApplies, hasOnboardElrs, isElrs, SERIAL_RX_PROTOCOLS, SPI_RX_PROTOCOLS } from "./receiver";
import { reflashBlock } from "./reflash";
import { isSupportedFirmware, MIN_SUPPORTED_FIRMWARE } from "./version";

/** A validation problem; `key` is an i18n key under `validation.*`. */
export interface Issue {
  key: string;
  params?: Record<string, string | number>;
}

export interface StepGuard {
  /** Prerequisites to show the step's content at all. */
  canEnter: (s: WizardState) => boolean;
  /** Problems that block "Next". Empty = OK. */
  validate: (s: WizardState) => Issue[];
}

const connected = (s: WizardState) => s.fc !== null;
export const firmwareOk = (s: WizardState): boolean =>
  s.fc !== null && isSupportedFirmware(s.fc.fcVariant, s.fc.firmwareVersion) && (s.ui.flashed || s.ui.firmwareDecision === "keep");
const configured = (s: WizardState) => connected(s) && firmwareOk(s) && s.ports.length > 0;

/** Issues for one port role (shared by the receiver / vtx / gps / esc steps). */
export function portIssues(s: WizardState, role: PortRole): Issue[] {
  if (!neededRoles(s.model, s.ui).includes(role)) return [];
  const name = getRolePort(s.model, role);
  if (name === null) return [{ key: "validation.portMissing", params: { role } }];
  const port = s.ports.find((p) => p.name === name);
  if (!port || exclusionReason(port, role, s.model, s.ui) !== null) return [{ key: "validation.portConflict", params: { role, port: name } }];
  return [];
}

export function validateSwitches(s: WizardState): Issue[] {
  const issues: Issue[] = [];
  const sw = s.model.switches;
  const arms = sw.filter((m) => m.mode === MODES.arm);
  if (arms.length === 0) issues.push({ key: "validation.armRequired" });
  const seen = new Set<string>();
  for (const m of sw) {
    if (seen.has(m.mode)) issues.push({ key: "validation.modeDuplicate", params: { mode: m.mode } });
    seen.add(m.mode);
    // Only stable names go to the config engine (config::modes maps them per firmware).
    const k = modeKeyOf(m.mode);
    const fw = s.fc?.firmwareVersion ?? s.model.fc.firmwareVersion;
    if (!k || m.mode !== MODES[k]) issues.push({ key: "validation.modeUnknown", params: { mode: m.mode } });
    else if (!modeAvailable(k, fw, catalogState.catalog?.modes)) {
      const min = MODE_MIN_FIRMWARE[k];
      issues.push(min ? { key: "validation.modeNeedsFirmware", params: { mode: m.mode, version: min } } : { key: "validation.modeNotInFirmware", params: { mode: m.mode, version: fw || "?" } });
    }
    const badRange =
      m.rangeMin < RANGE_MIN || m.rangeMax > RANGE_MAX || m.rangeMin >= m.rangeMax || m.rangeMin % RANGE_STEP !== 0 || m.rangeMax % RANGE_STEP !== 0;
    if (badRange) issues.push({ key: "validation.modeRangeInvalid", params: { mode: m.mode } });
    if (!Number.isInteger(m.auxChannel) || m.auxChannel < 0 || m.auxChannel >= MAX_AUX) issues.push({ key: "validation.auxInvalid", params: { mode: m.mode } });
  }
  // A range covering the whole stick travel would arm at power-up.
  for (const a of arms) if (a.rangeMin <= 1000 && a.rangeMax >= 2000) issues.push({ key: "validation.armAlwaysOn" });
  // ExpressLRS needs ARM on AUX1 (it switches packet rate / telemetry on it).
  if (s.model.receiver.protocol === "EXPRESSLRS" && arms.some((a) => a.auxChannel !== 0)) issues.push({ key: "validation.elrsArmAux1" });
  // Two modes on the same channel AND overlapping range would fire together.
  for (let i = 0; i < sw.length; i++)
    for (let j = i + 1; j < sw.length; j++) {
      const a = sw[i]!;
      const b = sw[j]!;
      if (a.auxChannel === b.auxChannel && a.rangeMin < b.rangeMax && b.rangeMin < a.rangeMax && (a.mode === MODES.arm || b.mode === MODES.arm))
        issues.push({ key: "validation.armOverlap", params: { mode: a.mode === MODES.arm ? b.mode : a.mode } });
    }
  // Prearm is a second, separate switch: on the arm switch it adds no safety.
  for (const p of sw.filter((m) => m.mode === MODES.prearm))
    if (arms.some((a) => a.auxChannel === p.auxChannel)) issues.push({ key: "validation.prearmSameSwitch" });
  // Turtle mode spins the motors backwards with DShot commands.
  for (const m of sw) {
    const k = modeKeyOf(m.mode);
    if (k && NEEDS_DSHOT.includes(k) && !s.model.esc.protocol.startsWith("DSHOT")) issues.push({ key: "validation.turtleNeedsDshot" });
  }
  if (s.ext.airmode === "SWITCH" && !sw.some((m) => m.mode === MODES.airMode)) issues.push({ key: "validation.airmodeSwitchMissing" });
  return issues;
}

/** Switches in the plan whose mode needs the accelerometer (ANGLE, HORIZON, ALTHOLD, POSHOLD, GPS RESCUE). */
export function accModeSwitches(s: WizardState): string[] {
  return s.model.switches.filter((m) => ACC_MODES.includes(modeKeyOf(m.mode)!)).map((m) => m.mode);
}

/**
 * Blocking: the plan turns the accelerometer off (sensors.acc = false) while a switch for
 * a mode that needs it is in the plan. The Calibration step offers the fixes.
 */
export function sensorIssues(s: WizardState): Issue[] {
  const modes = accModeSwitches(s);
  const out: Issue[] = s.ext.sensors?.acc === false && modes.length ? [{ key: "validation.accNeededByModes", params: { modes: modes.join(", ") } }] : [];
  if (s.ext.boardAlignment && !validAlignment(s.ext.boardAlignment)) out.push({ key: "validation.alignmentInvalid" });
  return out;
}

/**
 * The flight controller may hold unsaved RAM changes after an apply that failed half-way.
 * Acc calibration saves to EEPROM (with any pending RAM changes); Device refuses it then
 * (PENDING_UNSAVED_CHANGES, cleared only by "Reboot board"), so this is only a hint.
 * A running apply is calibrationBlock's "applying".
 */
export const mayHaveUnsavedChanges = (s: WizardState): boolean => s.ui.applyStatus === "failed";

/**
 * Config rule: acc calibration only BEFORE config_plan or AFTER config_apply returned, never
 * in between (the plan was made against the board as it was; calibrating saves the board's
 * RAM config). "planPending" = Review made a plan that was not applied yet; "applying" =
 * config_apply is running. The Calibration step drops a pending plan when it is entered
 * (Review makes a new one on its next visit), so "planPending" is a safety net.
 */
export type CalibrationBlock = "planPending" | "applying" | null;
export function calibrationBlock(s: WizardState): CalibrationBlock {
  if (s.ui.applyStatus === "applying" || s.ui.applyStatus === "generating") return "applying";
  if (s.ui.script !== null && s.ui.applyStatus === "idle") return "planPending";
  return null;
}

/** Non-blocking warnings on the Calibration step (shown there and on Review). */
export function calibrationWarnings(s: WizardState): Issue[] {
  const out: Issue[] = [];
  const explicitOff = s.ext.sensors?.acc === false;
  const accOn = s.ext.sensors?.acc ?? (s.ui.sensors ? s.ui.sensors.acc.present && s.ui.sensors.acc.enabled : true);
  // acc:false + those modes is a blocking error (sensorIssues); here only a board without one.
  if (!accOn && !explicitOff && accModeSwitches(s).length) out.push({ key: "calibration.warnAngleNoAcc" });
  if (!accOn && s.model.gps.enabled) out.push({ key: "calibration.warnGpsNoAcc" });
  if (s.ext.sensors?.baro === false && s.model.switches.some((m) => BARO_MODES.includes(modeKeyOf(m.mode)!))) out.push({ key: "calibration.warnAltholdNoBaro" });
  return out;
}

export const guards: Record<string, StepGuard> = {
  welcome: { canEnter: () => true, validate: () => [] },
  connect: {
    canEnter: () => true,
    validate: (s) => (connected(s) ? [] : [{ key: "validation.noBoard" }]),
  },
  firmware: {
    canEnter: connected,
    validate: (s) => {
      if (!s.fc) return [{ key: "validation.noBoard" }];
      if (!isSupportedFirmware(s.fc.fcVariant, s.fc.firmwareVersion))
        return [{ key: "validation.firmwareUnsupported", params: { min: MIN_SUPPORTED_FIRMWARE } }];
      return firmwareOk(s) ? [] : [{ key: "validation.firmwareDecide" }];
    },
  },
  components: {
    canEnter: configured,
    validate: (s) => {
      const issues: Issue[] = [];
      if (s.ports.length === 0) issues.push({ key: "validation.noConfig" });
      if (s.model.receiver.kind === "NONE") issues.push({ key: "validation.receiverRequired" });
      if (s.model.receiver.kind === "SPI" && !hasOnboardElrs(s.fc)) issues.push({ key: "validation.noOnboardReceiver" });
      if (s.model.vtx.kind !== "NONE" && !s.model.vtx.protocol) issues.push({ key: "validation.vtxProtocolRequired" });
      return issues;
    },
  },
  receiver: {
    canEnter: configured,
    validate: (s) => {
      const r = s.model.receiver;
      const issues: Issue[] = [];
      const allowed = r.kind === "SPI" ? SPI_RX_PROTOCOLS : SERIAL_RX_PROTOCOLS;
      if (!r.protocol || !allowed.includes(r.protocol)) issues.push({ key: "validation.rxProtocolRequired" });
      // The board's build lacks this receiver (known only when its build options are; unknown never blocks).
      const m = s.ui.rxMissing;
      if (m && m.kind === r.kind && m.protocol === r.protocol && m.missing.length) {
        // Say plainly when the update can't be offered: then another protocol is the only way on.
        const block = reflashBlock(s);
        const key = !block ? "validation.rxOptionMissing" : block === "applied" ? "validation.rxOptionMissingApplied" : "validation.rxOptionMissingNoUpdate";
        issues.push({ key, params: { protocol: r.protocol } });
      }
      return [...issues, ...portIssues(s, "receiver")];
    },
  },
  binding: {
    canEnter: configured,
    validate: (s) => {
      if (!bindingApplies(s)) return [];
      const r = s.model.receiver;
      if (r.kind === "SERIAL" && !s.ui.crsfFamily) return [{ key: "validation.crsfFamilyRequired" }];
      if (!isElrs(s)) return [];
      if (!s.ui.elrsBinding) return [{ key: "validation.bindingRequired" }];
      if (r.kind === "SPI" && s.ui.elrsBinding === "phrase" && r.elrsUid?.length !== 6) return [{ key: "validation.phraseNotSet" }];
      return [];
    },
  },
  vtx: {
    canEnter: configured,
    validate: (s) => {
      const v = s.model.vtx;
      const issues: Issue[] = [];
      if (!v.protocol) issues.push({ key: "validation.vtxProtocolRequired" });
      if (v.kind === "DIGITAL" && !v.digitalSystem) issues.push({ key: "validation.digitalSystemRequired" });
      return [...issues, ...portIssues(s, "vtx")];
    },
  },
  gps: {
    canEnter: configured,
    validate: (s) => [...(s.model.gps.provider ? [] : [{ key: "validation.gpsProviderRequired" }]), ...portIssues(s, "gps")],
  },
  esc: {
    canEnter: configured,
    validate: (s) => {
      const e = s.model.esc;
      const issues: Issue[] = [];
      if (e.bidirectionalDshot && !e.protocol.startsWith("DSHOT")) issues.push({ key: "validation.bidirNeedsDshot" });
      if (e.bidirectionalDshot && (e.motorPoles === null || e.motorPoles < 2 || e.motorPoles % 2 !== 0))
        issues.push({ key: "validation.motorPolesInvalid" });
      issues.push(...portIssues(s, "escTelemetry"));
      // Motor order/direction confirmations are optional (recommended, not gating): the
      // motor test needs the battery connected, and some users skip it or cannot do it.
      return issues;
    },
  },
  switches: { canEnter: configured, validate: validateSwitches },
  calibration: {
    canEnter: configured,
    // Calibration and the orientation check are recommended, not gating (the board may
    // already be calibrated, and some backends cannot calibrate yet). Running = wait.
    validate: (s) => [...(s.ui.accCalibration === "running" ? [{ key: "validation.calibrationRunning" }] : []), ...sensorIssues(s)],
  },
  beepers: {
    canEnter: configured,
    validate: (s) => (s.ui.beeperMode === null ? [{ key: "validation.beepersChoose" }] : []),
  },
  osd: { canEnter: configured, validate: validateOsd },
  review: {
    canEnter: configured,
    validate: (s) => (s.ui.script ? [] : [{ key: s.ui.planUnavailable ? "validation.planUnavailable" : "validation.notGenerated" }]),
  },
  apply: {
    canEnter: (s) => configured(s) && s.ui.script !== null,
    validate: (s) => (s.ui.applyStatus === "verified" ? [] : [{ key: "validation.notApplied" }]),
  },
  done: { canEnter: (s) => s.ui.applyStatus === "verified", validate: () => [] },
};

export function guardFor(id: string): StepGuard {
  const g = guards[id];
  if (!g) throw new Error(`No guard for wizard step: ${id}`);
  return g;
}

export function canLeave(id: string, s: WizardState): boolean {
  return guardFor(id).validate(s).length === 0;
}

/** Jump to `targetId`? Going back is always allowed; forward needs every step before it leavable. */
export function canGoTo(targetId: string, currentId: string, s: WizardState, steps: readonly StepDef[]): boolean {
  const target = steps.findIndex((x) => x.id === targetId);
  const current = steps.findIndex((x) => x.id === currentId);
  if (target < 0) return false;
  if (target <= current) return true;
  for (let i = 0; i < target; i++) if (!canLeave(steps[i]!.id, s)) return false;
  return guardFor(targetId).canEnter(s);
}

/**
 * OSD: the video system must be known (and HD needs firmware with an HD OSD), and
 * "set up" needs the layout loaded. A digital VTX with an analog OSD is only a warning
 * (shown on the step).
 */
export function validateOsd(s: WizardState): Issue[] {
  const system = effectiveSystem(s.model, s.osd, s.fc);
  // "Keep the board's OSD" needs no video system (nothing OSD-related is sent).
  if (s.osd.mode === "keep") return [];
  if (!system) return [{ key: "validation.osdSystem" }];
  if (system === "HD" && hdSupport(s.fc, s.osd) === false) return [{ key: "validation.osdHdUnsupported" }];
  if (s.osd.mode === null) return [{ key: "validation.osdChoose" }];
  if (s.osd.mode === "choose" && !s.osd.layout) return [{ key: "validation.osdLayout" }];
  return [];
}
