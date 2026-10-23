// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * "Keep my settings" after a firmware update: UI-local adapter for Config's
 * `config_restore_after_flash(backupId) -> RestoreReport` (shared PR #92, ARCHITECTURE §5.2).
 *
 * TODO(shared #92): the types below mirror #92's src/types/model.ts additions exactly
 * (RestoreReport, RestoreFailure, RestoreSkip, RestoreSkipReason, the four restore
 * ConfigErrorReasons with {backupId, path?, stage?}, ConfigProgress.line/total). Until #92 is
 * on main the command is not in CommandMap, so it is called by name through the same
 * backend seam (`call`), which keeps the demo fallback and the mixed-backend write guard
 * (api/fallback.ts WRITE_DEPENDENCIES). Once #92 lands: import the types from
 * types/model and call api.config.restoreAfterFlash instead; nothing else changes.
 *
 * Every refusal or failure means the same thing to the user: the flash itself succeeded,
 * the settings were not put back, and the backup is still saved (Backups list).
 */
import { call, type CommandName } from "../api/commands";
import type { ConfigProgress, ConfigStage } from "../types/model";
import { toUiError, type UiError } from "../errors/codes";
import { firmwareLine } from "./firmwareSupport";

export const RESTORE_COMMAND = "config_restore_after_flash";

export type RestoreSkipReason =
  | "COMMENT"
  | "SESSION_CONTROL"
  | "BOARD_IDENTITY"
  | "HARDWARE_LINE_CHANGED"
  | "REMOVED_ON_TARGET"
  | "NOT_IN_BUILD"
  | "SEMANTICS_CHANGED"
  | "NO_EQUIVALENT_MODE"
  | "PORT_NOT_AVAILABLE"
  | "OTHER_BATTERY_PROFILE";
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
/** `line`: the backup line (secret values masked). `error`: the FC's answer, shown verbatim. */
export interface RestoreFailure {
  line: string;
  error: string;
}
/** `detail`: the backend's own short explanation (English), e.g. "removed in 2025.12". */
export interface RestoreSkip {
  line: string;
  reason: RestoreSkipReason;
  detail: string;
}
export interface RestoreReport {
  applied: number;
  failed: RestoreFailure[];
  skipped: RestoreSkip[];
  warnings: string[];
  saved: boolean;
  lineChanged: boolean;
  fromVersion: string;
  toVersion: string;
}

/** CONFIG details.reason values config_restore_after_flash refuses or fails with. */
export const RESTORE_REFUSAL_REASONS = ["BOARD_UID_MISMATCH", "BACKUP_NOT_DIFF_ALL", "BACKUP_UNREADABLE", "BOARD_NOT_RECONNECTED"] as const;
export type RestoreRefusalReason = (typeof RESTORE_REFUSAL_REASONS)[number];
export interface RestoreErrorDetails {
  reason: RestoreRefusalReason;
  backupId: string;
  path?: string;
  /** BOARD_NOT_RECONNECTED only: absent = nothing sent; RESTORING = nothing saved; SAVING = unknown; REBOOTING = saved, not checked. */
  stage?: ConfigStage | "RESTORING";
}
/** config://progress with #92's RESTORING stage and line counter. */
export type RestoreProgress = Omit<ConfigProgress, "stage"> & { stage: ConfigStage | "RESTORING"; line?: number; total?: number };

/**
 * Skips that are always there by design and say nothing about the user's settings: comments,
 * batch start/end/save, and the board's own name/ID lines. Never listed to the user.
 */
export const HIDDEN_SKIPS: RestoreSkipReason[] = ["COMMENT", "SESSION_CONTROL", "BOARD_IDENTITY"];
export const visibleSkips = (r: RestoreReport): RestoreSkip[] => r.skipped.filter((s) => !HIDDEN_SKIPS.includes(s.reason));

type AnyCall = (cmd: CommandName, args: { backupId: string }) => Promise<unknown>;

export async function restoreAfterFlash(backupId: string): Promise<RestoreReport> {
  return (await (call as unknown as AnyCall)(RESTORE_COMMAND as CommandName, { backupId })) as RestoreReport;
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

/** Restoring settings saved on another firmware line (e.g. 4.5 -> 2025.12). */
export function crossLine(boardVersion: string | null | undefined, target: string | null | undefined): boolean {
  const a = firmwareLine(boardVersion);
  const b = firmwareLine(target);
  return !!a && !!b && a !== b;
}
