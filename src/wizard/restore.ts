// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * "Keep my settings" after a firmware update: UI helpers around Config's
 * `api.config.restoreAfterFlash(backupId) -> RestoreReport` (shared contract #92,
 * ARCHITECTURE §5.2). The types come from types/model.ts; the call goes through the usual
 * backend seam, so the demo fallback and the mixed-backend write guard apply
 * (api/fallback.ts WRITE_DEPENDENCIES).
 *
 * Every refusal or failure means the same thing to the user: the flash itself succeeded,
 * the settings were not put back, and the backup is still saved (Backups list).
 */
import { api } from "../api/commands";
import type { ConfigErrorDetails, ConfigErrorReason, ConfigProgress, RestoreReport, RestoreSkip, RestoreSkipReason } from "../types/model";
import { toUiError, type UiError } from "../errors/codes";
import { firmwareLine } from "./firmwareSupport";

export type { RestoreFailure, RestoreReport, RestoreSkip, RestoreSkipReason } from "../types/model";

/** The command name (the demo fallback's WRITE_DEPENDENCIES key, mocks). */
export const RESTORE_COMMAND = "config_restore_after_flash";

export const RESTORE_SKIP_REASONS: RestoreSkipReason[] = [
  "COMMENT",
  "SESSION_CONTROL",
  "BOARD_IDENTITY",
  "HARDWARE_LINE_CHANGED",
  "REMOVED_ON_TARGET",
  "NOT_IN_BUILD",
  "SEMANTICS_CHANGED",
  "NO_EQUIVALENT_MODE",
  "PORT_NOT_AVAILABLE",
  "OTHER_BATTERY_PROFILE",
];

/** CONFIG details.reason values config_restore_after_flash refuses or fails with. */
export const RESTORE_REFUSAL_REASONS = ["BOARD_UID_MISMATCH", "BACKUP_NOT_DIFF_ALL", "BACKUP_UNREADABLE", "BOARD_NOT_RECONNECTED"] as const satisfies readonly ConfigErrorReason[];
export type RestoreRefusalReason = (typeof RESTORE_REFUSAL_REASONS)[number];
/** The CONFIG details of a restore refusal (always with backupId on this path). */
export type RestoreErrorDetails = ConfigErrorDetails & { reason: RestoreRefusalReason };
/** config://progress (RESTORING carries line / total). */
export type RestoreProgress = ConfigProgress;

/**
 * Skips that are always there by design and say nothing about the user's settings: comments,
 * batch start/end/save, and the board's own name/ID lines. Never listed to the user.
 */
export const HIDDEN_SKIPS: RestoreSkipReason[] = ["COMMENT", "SESSION_CONTROL", "BOARD_IDENTITY"];
export const visibleSkips = (r: RestoreReport): RestoreSkip[] => r.skipped.filter((s) => !HIDDEN_SKIPS.includes(s.reason));

/**
 * Text for a skipped line: `restore.skip.<reason>` for the reasons #92/#97 send, null for a
 * reason this UI does not know yet (shown as a generic sentence plus the raw code).
 */
export function skipKey(reason: string): string | null {
  return (RESTORE_SKIP_REASONS as string[]).includes(reason) ? `restore.skip.${reason}` : null;
}

/**
 * The restore's `warnings` are English sentences from Config (#97 config/restore.rs). The ones
 * it can send are recognised here and shown in the user's language (`restore.note.<kind>`);
 * anything else is shown as sent. Values (versions, setting / port / board names) are passed
 * through as-is.
 */
export type RestoreNoteKind =
  | "CONVERTED"
  | "D_TERM"
  | "BATTERY_INTO_PROFILE"
  | "OTHER_BATTERY_PROFILE"
  | "SERIAL_FUNCTIONS_REMOVED"
  | "MATCHED_BY_NAME"
  | "READ_BACK_FAILED"
  | "READ_BACK_DIFFERS";
export const RESTORE_NOTE_KINDS: RestoreNoteKind[] = [
  "CONVERTED",
  "D_TERM",
  "BATTERY_INTO_PROFILE",
  "OTHER_BATTERY_PROFILE",
  "SERIAL_FUNCTIONS_REMOVED",
  "MATCHED_BY_NAME",
  "READ_BACK_FAILED",
  "READ_BACK_DIFFERS",
];
export interface RestoreNote {
  kind: RestoreNoteKind | null;
  params: Record<string, string>;
  /** The sentence as Config sent it (shown when not recognised, and as the technical detail). */
  raw: string;
}
const NOTE_PATTERNS: [RestoreNoteKind, RegExp, string[]][] = [
  ["CONVERTED", /^Settings were converted from Betaflight (\S+) to (\S+); check the skipped lines\.?$/, ["from", "to"]],
  ["D_TERM", /^D-term settings were not restored: their meaning changed in 2025\.12\b/, []],
  ["BATTERY_INTO_PROFILE", /^Battery settings \((.+)\) were restored into battery profile (\d+), the selected one; battery profiles .+ keep their defaults\.?$/, ["settings", "profile"]],
  ["OTHER_BATTERY_PROFILE", /^Only battery profile (\d+) was restored: Betaflight (\S+) has one set of battery settings\.?$/, ["profile", "to"]],
  ["SERIAL_FUNCTIONS_REMOVED", /^Serial functions that do not exist on Betaflight (\S+) were removed: (.+?)\.?$/, ["to", "ports"]],
  ["MATCHED_BY_NAME", /^The board's unique id is unknown; the backup was matched by board name \((.+)\) only\.?$/, ["board"]],
  ["READ_BACK_FAILED", /^The restored settings could not be read back: (.*)$/, ["error"]],
  ["READ_BACK_DIFFERS", /^Read back: (?:feature )?(\S+) (?:is |differs )/, ["name"]],
];
/** Unicode isolates, so an LTR value (a version, `UART6 (mask 64)`) sits cleanly in Hebrew text. */
const isolate = (v: string) => `\u2068${v}\u2069`;
export function restoreNote(raw: string): RestoreNote {
  for (const [kind, re, names] of NOTE_PATTERNS) {
    const m = re.exec(raw.trim());
    if (m) return { kind, raw, params: Object.fromEntries(names.map((n, i) => [n, isolate(m[i + 1] ?? "")])) };
  }
  return { kind: null, raw, params: {} };
}

export async function restoreAfterFlash(backupId: string): Promise<RestoreReport> {
  return api.config.restoreAfterFlash(backupId);
}

/** Why the settings were not put back, as a key under `restore.refused.*`. */
export type RefusalKind = RestoreRefusalReason | "UNSUPPORTED_FIRMWARE" | "NOT_AVAILABLE" | "GENERIC";

export function refusalKind(e: UiError): RefusalKind {
  const reason = (e.details as { reason?: unknown } | undefined)?.reason;
  if (typeof reason === "string" && (RESTORE_REFUSAL_REASONS as readonly string[]).includes(reason)) return reason as RestoreRefusalReason;
  if (e.code === "UNSUPPORTED_FIRMWARE") return "UNSUPPORTED_FIRMWARE";
  // The backend can't restore yet (stub, or the demo guard refused to fake it for a real board).
  if (e.code === "NOT_IMPLEMENTED" || e.code === "MIXED_BACKEND") return "NOT_AVAILABLE";
  return "GENERIC";
}

/** BOARD_NOT_RECONNECTED: how far it got (`restore.refusedStage.*`), null when not reported. */
export function refusalStage(e: UiError): "NOTHING_SENT" | "RESTORING" | "SAVING" | "REBOOTING" | null {
  const d = e.details as Partial<RestoreErrorDetails> | undefined;
  if (d?.reason !== "BOARD_NOT_RECONNECTED") return null;
  if (!d.stage) return "NOTHING_SENT";
  return d.stage === "RESTORING" || d.stage === "SAVING" || d.stage === "REBOOTING" ? d.stage : null;
}

/** Can "Try again" help? Not for a backup that belongs to another board or isn't a full copy. */
export function canRetryRestore(e: UiError): boolean {
  const k = refusalKind(e);
  return k === "BOARD_NOT_RECONNECTED" || k === "GENERIC";
}

/** Outcome of the restore part of an update. */
export interface RestoreOutcome {
  report: RestoreReport | null;
  /** Set when the settings were not put back (the flash still succeeded). */
  error: UiError | null;
  /** The backup the restore used / would have used. */
  backupId: string | null;
}

export const restoreFailed = (e: unknown, backupId: string | null): RestoreOutcome => ({ report: null, error: toUiError(e), backupId });

/**
 * Betaflight's stored config version (EEPROM_CONF_VERSION) per line (ARCHITECTURE §6 "Settings
 * after a normal flash"): when it changes, the new firmware resets every setting at boot, even
 * without a full erase. 4.5 and 2025.12 share 177.
 */
const CONF_VERSION: Readonly<Record<string, number>> = { "4.4": 175, "4.5": 177, "2025.12": 177, "2026.6": 178 };

/** A normal flash from `boardVersion` to `target` starts from default settings (different lines, not 4.5 <-> 2025.12; unknown lines count as resetting). */
export function lineResetsSettings(boardVersion: string | null | undefined, target: string | null | undefined): boolean {
  const a = firmwareLine(boardVersion);
  const b = firmwareLine(target);
  if (!a || !b || a === b) return false;
  const ca = CONF_VERSION[a];
  const cb = CONF_VERSION[b];
  return ca === undefined || cb === undefined || ca !== cb;
}

/** Restoring settings saved on another firmware line (e.g. 4.5 -> 2025.12). */
export function crossLine(boardVersion: string | null | undefined, target: string | null | undefined): boolean {
  const a = firmwareLine(boardVersion);
  const b = firmwareLine(target);
  return !!a && !!b && a !== b;
}
