// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * AppError.code -> plain-language guidance with concrete next actions.
 * Text lives in the locales under `errors.<CODE>.{title,body,step1..n}`.
 *
 * Covers today's error.rs codes AND the agreed upcoming ones (BOARD_NOT_FOUND,
 * PORT_BUSY, DRIVER_MISSING, DFU_NOT_FOUND, FLASH_FAILED, VERIFY_MISMATCH), so the
 * detailed guidance works the moment the backend starts sending them.
 */
import type { AppError } from "../types/model";
import { UPCOMING_ERROR_CODES } from "../api/upcoming";

export const CURRENT_ERROR_CODES = ["NOT_IMPLEMENTED", "NOT_CONNECTED", "SERIAL", "MSP", "CLI", "USB", "FIRMWARE", "CONFIG", "DRIVER", "IO"] as const;
export const KNOWN_ERROR_CODES = [...CURRENT_ERROR_CODES, ...UPCOMING_ERROR_CODES, "UNKNOWN"] as const;
export type ErrorCode = (typeof KNOWN_ERROR_CODES)[number];

export type GuidanceActionId = "retry" | "reconnect" | "retryFlash" | "installDriver" | "editPorts" | "reapply" | "openLink";

export interface GuidanceAction {
  id: GuidanceActionId;
  labelKey: string;
  primary?: boolean;
  href?: string;
}

export interface Guidance {
  code: ErrorCode;
  titleKey: string;
  bodyKey: string;
  stepKeys: string[];
  actions: GuidanceAction[];
  severity: "error" | "warning";
}

/** Help links. All must stay inside the opener capability scope (see src/api/opener.ts). */
export const HELP_LINKS = {
  firmwareInstall: "https://betaflight.com/docs/wiki/getting-started/firmware-installation",
  issues: "https://github.com/eligreenmac/betaflight-setup-wizard/issues",
} as const;

/** Number of `stepN` keys per code (kept in sync with the locales by tests/i18n.test.ts). */
export const STEP_COUNTS: Record<ErrorCode, number> = {
  NOT_IMPLEMENTED: 1,
  NOT_CONNECTED: 2,
  SERIAL: 2,
  MSP: 2,
  CLI: 1,
  USB: 2,
  FIRMWARE: 2,
  CONFIG: 1,
  DRIVER: 2,
  IO: 1,
  BOARD_NOT_FOUND: 4,
  PORT_BUSY: 2,
  DRIVER_MISSING: 2,
  DFU_NOT_FOUND: 3,
  FLASH_FAILED: 3,
  VERIFY_MISMATCH: 2,
  UNKNOWN: 1,
};

const retry: GuidanceAction = { id: "retry", labelKey: "actions.tryAgain", primary: true };
const reconnect: GuidanceAction = { id: "reconnect", labelKey: "actions.reconnect", primary: true };
const retryFlash: GuidanceAction = { id: "retryFlash", labelKey: "actions.retryFlash", primary: true };
const installDriver: GuidanceAction = { id: "installDriver", labelKey: "actions.installDriver", primary: true };
const firmwareHelp: GuidanceAction = { id: "openLink", labelKey: "actions.openFirmwareHelp", href: HELP_LINKS.firmwareInstall };
const reportIssue: GuidanceAction = { id: "openLink", labelKey: "actions.reportIssue", href: HELP_LINKS.issues };

const ACTIONS: Record<ErrorCode, GuidanceAction[]> = {
  NOT_IMPLEMENTED: [reportIssue],
  NOT_CONNECTED: [reconnect],
  SERIAL: [retry],
  MSP: [reconnect],
  CLI: [retry, reportIssue],
  USB: [retryFlash, firmwareHelp],
  FIRMWARE: [retry, firmwareHelp],
  CONFIG: [{ id: "editPorts", labelKey: "actions.editPorts", primary: true }, reportIssue],
  DRIVER: [installDriver, firmwareHelp],
  IO: [retry, reportIssue],
  BOARD_NOT_FOUND: [retry],
  PORT_BUSY: [retry],
  DRIVER_MISSING: [installDriver, firmwareHelp, { id: "retry", labelKey: "actions.tryAgain" }],
  DFU_NOT_FOUND: [retryFlash, { id: "installDriver", labelKey: "actions.installDriver" }, firmwareHelp],
  FLASH_FAILED: [retryFlash, firmwareHelp],
  VERIFY_MISMATCH: [{ id: "reapply", labelKey: "actions.reapply", primary: true }],
  UNKNOWN: [retry, reportIssue],
};

export function isKnownCode(code: string): code is ErrorCode {
  return (KNOWN_ERROR_CODES as readonly string[]).includes(code);
}

export function guidanceFor(code: string): Guidance {
  const c: ErrorCode = isKnownCode(code) ? code : "UNKNOWN";
  return {
    code: c,
    titleKey: `errors.${c}.title`,
    bodyKey: `errors.${c}.body`,
    stepKeys: Array.from({ length: STEP_COUNTS[c] }, (_, i) => `errors.${c}.step${i + 1}`),
    actions: ACTIONS[c],
    severity: c === "VERIFY_MISMATCH" || c === "NOT_IMPLEMENTED" ? "warning" : "error",
  };
}

/** Normalise anything a command can reject with into the AppError shape. */
export function toAppError(e: unknown): AppError {
  if (e && typeof e === "object" && "code" in e && typeof (e as AppError).code === "string") {
    const ae = e as AppError;
    return { code: ae.code, message: String(ae.message ?? "") };
  }
  if (e instanceof Error) return { code: "UNKNOWN", message: e.message };
  return { code: "UNKNOWN", message: typeof e === "string" ? e : JSON.stringify(e) };
}

export const isNotImplemented = (e: unknown): boolean => toAppError(e).code === "NOT_IMPLEMENTED";
