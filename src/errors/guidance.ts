// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UiError.code -> plain-language guidance with concrete next actions
 * (codes from drafts DESIGN.md §6). Text lives in the locales under
 * `errors.<CODE>.{title,body,step1..n}`; tests/i18n.test.ts checks every key exists.
 */
import { ALL_ERROR_CODES, isErrorCode, toUiError, type ErrorCode, type UiError } from "./codes";

export { isNotImplemented, toUiError, type UiError } from "./codes";

export type GuidanceActionId =
  | "retry"
  | "reconnect"
  | "retryFlash"
  | "retryFullErase"
  | "installDriver"
  | "goFirmware"
  | "editPorts"
  | "editComponents"
  | "openLink";

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
  /** The backend message says exactly what went wrong (e.g. which safety rule refused); show it, not only in "details". */
  showMessage: boolean;
}

/** Help links. All must stay inside the opener capability scope (src/api/links.ts). */
export const HELP_LINKS = {
  firmwareInstall: "https://betaflight.com/docs/wiki/getting-started/firmware-installation",
  issues: "https://github.com/eligreenmac/betaflight-setup-wizard/issues",
} as const;

const retry: GuidanceAction = { id: "retry", labelKey: "actions.tryAgain", primary: true };
const retry2: GuidanceAction = { id: "retry", labelKey: "actions.tryAgain" };
const reconnect: GuidanceAction = { id: "reconnect", labelKey: "actions.reconnect", primary: true };
const retryFlash: GuidanceAction = { id: "retryFlash", labelKey: "actions.retryFlash", primary: true };
const installDriver: GuidanceAction = { id: "installDriver", labelKey: "actions.installDriver", primary: true };
const goFirmware: GuidanceAction = { id: "goFirmware", labelKey: "actions.goFirmware", primary: true };
const firmwareHelp: GuidanceAction = { id: "openLink", labelKey: "actions.openFirmwareHelp", href: HELP_LINKS.firmwareInstall };
const reportIssue: GuidanceAction = { id: "openLink", labelKey: "actions.reportIssue", href: HELP_LINKS.issues };

interface Entry {
  steps: number;
  actions: GuidanceAction[];
  warning?: boolean;
  showMessage?: boolean;
}

const TABLE: Record<ErrorCode, Entry> = {
  BOARD_NOT_FOUND: { steps: 4, actions: [retry] },
  PORT_BUSY: { steps: 2, actions: [retry] },
  DISCONNECTED: { steps: 2, actions: [reconnect] },
  TIMEOUT: { steps: 2, actions: [retry] },
  SERIAL_ERROR: { steps: 2, actions: [retry] },
  DRIVER_MISSING: { steps: 2, actions: [installDriver, firmwareHelp, retry2] },
  USB_ERROR: { steps: 2, actions: [retry] },
  UNSUPPORTED_FIRMWARE: { steps: 2, actions: [goFirmware] },
  MSP_UNSUPPORTED: { steps: 1, actions: [goFirmware, reportIssue] },
  MSP_BAD_RESPONSE: { steps: 1, actions: [retry, reportIssue] },
  CLI_ERROR: { steps: 1, actions: [retry, reportIssue], showMessage: true },
  DFU_NOT_FOUND: { steps: 3, actions: [retryFlash, { id: "installDriver", labelKey: "actions.installDriver" }, firmwareHelp] },
  FLASH_FAILED: { steps: 3, actions: [retryFlash, firmwareHelp] },
  VERIFY_MISMATCH: { steps: 2, actions: [{ id: "retryFullErase", labelKey: "actions.retryFullErase", primary: true }, firmwareHelp] },
  READ_PROTECTED: { steps: 2, actions: [firmwareHelp, reportIssue] },
  REPLUG_REQUIRED: { steps: 2, actions: [retryFlash] },
  FIRMWARE_TOO_LARGE: { steps: 1, actions: [reportIssue] },
  INVALID_FIRMWARE: { steps: 1, actions: [retryFlash] },
  NO_FLASH_LAYOUT: { steps: 1, actions: [firmwareHelp, reportIssue] },
  NETWORK_ERROR: { steps: 2, actions: [retry] },
  BUILD_API_ERROR: { steps: 1, actions: [retry] },
  BUILD_FAILED: { steps: 1, actions: [retry, firmwareHelp], showMessage: true },
  IO_ERROR: { steps: 1, actions: [retry, reportIssue] },
  DATA_ERROR: { steps: 1, actions: [retry] },
  MOTOR_TEST_REFUSED: { steps: 1, actions: [], showMessage: true },
  BOARD_ARMED: { steps: 1, actions: [] },
  MOTOR_TEST_NOT_ACTIVE: { steps: 1, actions: [], warning: true },
  CANCELLED: { steps: 1, actions: [], warning: true },
  BUSY: { steps: 1, actions: [retry], warning: true },
  INVALID_ARGUMENT: { steps: 1, actions: [reportIssue] },
  INTERNAL: { steps: 1, actions: [reportIssue] },
  NOT_IMPLEMENTED: { steps: 1, actions: [reportIssue], warning: true },
  CONFIG_UNSUPPORTED: { steps: 2, actions: [{ id: "editComponents", labelKey: "actions.editComponents", primary: true }, reportIssue], showMessage: true },
  UNKNOWN: { steps: 1, actions: [retry, reportIssue] },
};

/** Number of `stepN` keys per code (kept in sync with the locales by tests/i18n.test.ts). */
export const STEP_COUNTS = Object.fromEntries(ALL_ERROR_CODES.map((c) => [c, TABLE[c].steps])) as Record<ErrorCode, number>;

export function guidanceFor(codeOrError: string | UiError): Guidance {
  const raw = typeof codeOrError === "string" ? codeOrError : codeOrError.code;
  const c: ErrorCode = isErrorCode(raw) ? raw : toUiError({ code: raw, message: "" }).code;
  const e = TABLE[c];
  return {
    code: c,
    titleKey: `errors.${c}.title`,
    bodyKey: `errors.${c}.body`,
    stepKeys: Array.from({ length: e.steps }, (_, i) => `errors.${c}.step${i + 1}`),
    actions: e.actions,
    severity: e.warning ? "warning" : "error",
    showMessage: !!e.showMessage,
  };
}
