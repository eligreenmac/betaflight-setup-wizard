// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UiError: the error shape every Tauri command will reject with
 * (drafts DESIGN.md §6, "Error model"): `{ code, category, message }`.
 * `code` is fine-grained and stable (only ever added), `category` is the broad
 * group used for generic handling, `message` is English detail for "details".
 *
 * TODO(shared): today's src-tauri/src/error.rs still sends the Phase 0 codes
 * (`AppError {code, message}`, codes SERIAL, MSP, ...). They are accepted here as
 * aliases until error.rs / model.ts adopt UiError. No contract change in this PR.
 */

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
 * unknown command maps to); it drives the "not available yet" fallbacks. UNKNOWN is
 * the catch-all for anything unrecognised.
 */
export const UI_ERROR_CODES = ["NOT_IMPLEMENTED", "UNKNOWN"] as const;
export type UiOnlyCode = (typeof UI_ERROR_CODES)[number];
export type ErrorCode = DesignErrorCode | UiOnlyCode;
export const ALL_ERROR_CODES: ErrorCode[] = [...DESIGN_ERROR_CODES, ...UI_ERROR_CODES];

/** Phase 0 error.rs codes -> DESIGN.md §6 codes. TODO(shared): drop once error.rs sends UiError. */
export const LEGACY_CODE_ALIASES: Record<string, ErrorCode> = {
  NOT_CONNECTED: "DISCONNECTED",
  SERIAL: "SERIAL_ERROR",
  MSP: "MSP_BAD_RESPONSE",
  CLI: "CLI_ERROR",
  USB: "USB_ERROR",
  FIRMWARE: "INVALID_FIRMWARE",
  CONFIG: "INVALID_ARGUMENT",
  DRIVER: "DRIVER_MISSING",
  IO: "IO_ERROR",
};

export interface UiError {
  code: ErrorCode;
  category: ErrorCategory;
  message: string;
}

export function isErrorCode(code: string): code is ErrorCode {
  return (ALL_ERROR_CODES as string[]).includes(code);
}

export function categoryOf(code: ErrorCode): ErrorCategory {
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
    const raw = e as { code: string; message?: unknown };
    const code: ErrorCode = isErrorCode(raw.code) ? raw.code : (LEGACY_CODE_ALIASES[raw.code] ?? "UNKNOWN");
    const message = String(raw.message ?? "");
    return { code, category: categoryOf(code), message: code === "UNKNOWN" && raw.code !== "UNKNOWN" ? `${raw.code}: ${message}` : message };
  }
  if (typeof e === "string" && /command .* not found|unknown command/i.test(e)) return uiError("NOT_IMPLEMENTED", e);
  if (e instanceof Error) return uiError("UNKNOWN", e.message);
  return uiError("UNKNOWN", typeof e === "string" ? e : JSON.stringify(e));
}

export const isNotImplemented = (e: unknown): boolean => toUiError(e).code === "NOT_IMPLEMENTED";
