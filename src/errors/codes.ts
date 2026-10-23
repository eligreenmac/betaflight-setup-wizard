// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UiError: the error shape every Tauri command will reject with
 * (drafts DESIGN.md §6, "Error model"): `{ code, category, message }`.
 * `code` is fine-grained and stable (only ever added), `category` is the broad
 * group used for generic handling, `message` is English detail for "details".
 *
 * error.rs sends `AppError {code, message, details?}` (#38): the DESIGN.md §6 codes
 * where the backend has them, plus the Phase 0 codes (SERIAL, MSP, ...), accepted here
 * as aliases. `details` (PORT_BUSY {port, heldBy}, DRIVER_MISSING {vid, pid, detail})
 * is kept on the UiError. There is no backend BUSY code; a busy port is PORT_BUSY.
 */

import type { DriverMissingDetails, PendingUnsavedChangesDetails, PortBusyDetails } from "../types/model";

export const ERROR_CATEGORIES = ["Connection", "Device", "Protocol", "Flash", "Network", "Io", "Safety", "Operation"] as const;
export type ErrorCategory = (typeof ERROR_CATEGORIES)[number];

/** The 31 codes of DESIGN.md §6, with their category. */
export const ERROR_CODE_CATEGORY = {
  BOARD_NOT_FOUND: "Connection",
  PORT_BUSY: "Connection",
  DISCONNECTED: "Connection",
  TIMEOUT: "Connection",
  SERIAL_ERROR: "Connection",
  DRIVER_MISSING: "Device",
  USB_ERROR: "Device",
  UNSUPPORTED_FIRMWARE: "Protocol",
  MSP_UNSUPPORTED: "Protocol",
  MSP_BAD_RESPONSE: "Protocol",
  CLI_ERROR: "Protocol",
  DFU_NOT_FOUND: "Flash",
  FLASH_FAILED: "Flash",
  VERIFY_MISMATCH: "Flash",
  READ_PROTECTED: "Flash",
  REPLUG_REQUIRED: "Flash",
  FIRMWARE_TOO_LARGE: "Flash",
  INVALID_FIRMWARE: "Flash",
  NO_FLASH_LAYOUT: "Flash",
  NETWORK_ERROR: "Network",
  BUILD_API_ERROR: "Network",
  BUILD_FAILED: "Network",
  IO_ERROR: "Io",
  DATA_ERROR: "Io",
  MOTOR_TEST_REFUSED: "Safety",
  BOARD_ARMED: "Safety",
  MOTOR_TEST_NOT_ACTIVE: "Safety",
  CANCELLED: "Operation",
  BUSY: "Operation",
  INVALID_ARGUMENT: "Operation",
  INTERNAL: "Operation",
} as const satisfies Record<string, ErrorCategory>;

export type DesignErrorCode = keyof typeof ERROR_CODE_CATEGORY;
export const DESIGN_ERROR_CODES = Object.keys(ERROR_CODE_CATEGORY) as DesignErrorCode[];

/**
 * UI-only codes: NOT_IMPLEMENTED is what the Phase 0 command stubs return (and what an
 * unknown command maps to); it drives the "not available yet" fallbacks.
 * CONFIG_UNSUPPORTED is the config engine's `CONFIG` error: the board cannot take part
 * of the setup, in the MVP mostly the receiver (an SPI protocol other than ExpressLRS,
 * or no onboard SPI receiver, ARCHITECTURE §5.1). MIXED_BACKEND is the demo-fallback
 * safety stop: a write would combine simulated and real data (api/fallback.ts), so
 * nothing was sent. "Reboot board" (device_reboot) failures, which the Calibration step
 * maps: REBOOT_NOT_CONFIRMED = not confirmed (PENDING_UNSAVED_CHANGES / MSP), press it again;
 * REBOOT_BOARD_LOST = the board did not come back (SERIAL / NOT_CONNECTED), check the cable
 * and connect again. CONNECT_REBOOT_NOT_CONFIRMED = device_connect answered
 * PENDING_UNSAVED_CHANGES (its reboot of a board left with unsaved changes was not
 * confirmed), try connecting again. A reconnect or replug never clears the flag (it lives
 * in the app; a battery-powered FC keeps its RAM). UNKNOWN is the catch-all.
 */
export const UI_ERROR_CODES = [
  "NOT_IMPLEMENTED",
  "CONFIG_UNSUPPORTED",
  "MIXED_BACKEND",
  "REBOOT_NOT_CONFIRMED",
  "REBOOT_BOARD_LOST",
  "CONNECT_REBOOT_NOT_CONFIRMED",
  "UNKNOWN",
] as const;
export type UiOnlyCode = (typeof UI_ERROR_CODES)[number];
/**
 * Backend codes outside the DESIGN.md §6 table.
 * device_acc_calibrate (#63 §2.3, in AppErrorCode): ACC_NOT_AVAILABLE {present, enabled} =
 * accelerometer missing or disabled; BOARD_MOVING {maxRateDps, limitDps, during} = not still
 * (during: the bad result is already stored); ACC_CALIBRATION_FAILED = the FC never finished.
 * PENDING_UNSAVED_CHANGES {reason} (#68/#69): device_acc_calibrate refuses (checked first)
 * while the FC's RAM may hold unsaved changes, because calibrating saves the whole RAM config.
 * Only a confirmed reboot clears it (api.device.reboot); device_reboot and device_connect
 * answer it when their reboot was not confirmed.
 */
export const PENDING_ERROR_CODES = {
  ACC_NOT_AVAILABLE: "Device",
  BOARD_MOVING: "Device",
  ACC_CALIBRATION_FAILED: "Device",
  PENDING_UNSAVED_CHANGES: "Safety",
} as const satisfies Record<string, ErrorCategory>;
export type PendingErrorCode = keyof typeof PENDING_ERROR_CODES;

export type ErrorCode = DesignErrorCode | UiOnlyCode | PendingErrorCode;
export const ALL_ERROR_CODES: ErrorCode[] = [...DESIGN_ERROR_CODES, ...UI_ERROR_CODES, ...(Object.keys(PENDING_ERROR_CODES) as PendingErrorCode[])];

/** error.rs codes -> DESIGN.md §6 / UI codes. UNSUPPORTED_FIRMWARE and NOT_IMPLEMENTED pass through as-is. */
export const LEGACY_CODE_ALIASES: Record<string, ErrorCode> = {
  NOT_CONNECTED: "DISCONNECTED",
  SERIAL: "SERIAL_ERROR",
  MSP: "MSP_BAD_RESPONSE",
  CLI: "CLI_ERROR",
  USB: "USB_ERROR",
  FIRMWARE: "INVALID_FIRMWARE",
  CONFIG: "CONFIG_UNSUPPORTED",
  DRIVER: "DRIVER_MISSING",
  IO: "IO_ERROR",
  // The UI only calls flasher_remove_read_protection with confirmErase=true, so this is a bug if seen.
  ERASE_CONFIRMATION_REQUIRED: "INTERNAL",
};

export interface UiError {
  code: ErrorCode;
  category: ErrorCategory;
  message: string;
  /** Structured extras from AppError (#38): PORT_BUSY {port, heldBy}, DRIVER_MISSING {vid, pid, detail}. */
  details?: PortBusyDetails | DriverMissingDetails | PendingUnsavedChangesDetails;
}

export const isPortBusyDetails = (d: unknown): d is PortBusyDetails =>
  !!d && typeof d === "object" && ((d as PortBusyDetails).heldBy === "APP" || (d as PortBusyDetails).heldBy === "EXTERNAL");

export function isErrorCode(code: string): code is ErrorCode {
  return (ALL_ERROR_CODES as string[]).includes(code);
}

export function categoryOf(code: ErrorCode): ErrorCategory {
  if (code === "MIXED_BACKEND") return "Safety";
  if (code === "REBOOT_NOT_CONFIRMED" || code === "REBOOT_BOARD_LOST" || code === "CONNECT_REBOOT_NOT_CONFIRMED") return "Connection";
  if (code in PENDING_ERROR_CODES) return PENDING_ERROR_CODES[code as PendingErrorCode];
  return code in ERROR_CODE_CATEGORY ? ERROR_CODE_CATEGORY[code as DesignErrorCode] : "Operation";
}

/** Build a UiError (mocks and UI-side checks raise exactly the backend's shape). */
export function uiError(code: ErrorCode, message: string): UiError {
  return { code, category: categoryOf(code), message };
}

/**
 * Normalise anything a command can reject with into UiError:
 * UiError as-is; Phase 0 AppError via aliases; Tauri's "command X not found"
 * string -> NOT_IMPLEMENTED; anything else -> UNKNOWN.
 */
export function toUiError(e: unknown): UiError {
  if (e && typeof e === "object" && "code" in e && typeof (e as { code: unknown }).code === "string") {
    const raw = e as { code: string; message?: unknown; details?: unknown };
    const code: ErrorCode = isErrorCode(raw.code) ? raw.code : (LEGACY_CODE_ALIASES[raw.code] ?? "UNKNOWN");
    const message = String(raw.message ?? "");
    const out: UiError = { code, category: categoryOf(code), message: code === "UNKNOWN" && raw.code !== "UNKNOWN" ? `${raw.code}: ${message}` : message };
    if (raw.details && typeof raw.details === "object") out.details = raw.details as UiError["details"];
    return out;
  }
  if (typeof e === "string" && /command .* not found|unknown command/i.test(e)) return uiError("NOT_IMPLEMENTED", e);
  if (e instanceof Error) return uiError("UNKNOWN", e.message);
  return uiError("UNKNOWN", typeof e === "string" ? e : JSON.stringify(e));
}

export const isNotImplemented = (e: unknown): boolean => toUiError(e).code === "NOT_IMPLEMENTED";
