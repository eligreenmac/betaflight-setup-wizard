// SPDX-License-Identifier: GPL-3.0-or-later
/** Backup listing (config_list_backups) and restore (config_restore). */
import { api } from "../api/commands";
import { isNotImplemented, uiError } from "../errors/codes";
import type { ApplyResult, Backup, FcSerialPort } from "../types/model";

export interface BackupList {
  backups: Backup[];
  /** "backend" = config_list_backups; "session" = only backups made in this session (older backend). */
  source: "backend" | "session";
}

const newestFirst = (a: Backup, b: Backup) => b.createdAt.localeCompare(a.createdAt);

/** Backups of the connected board only (matched by boardUid), newest first. */
export async function listBackups(boardUid: string | null | undefined, sessionBackups: Backup[]): Promise<BackupList> {
  const mine = (list: Backup[]) => (boardUid ? list.filter((b) => b.boardUid === boardUid) : []).sort(newestFirst);
  try {
    return { backups: mine([...(await api.config.listBackups())]), source: "backend" };
  } catch (e) {
    if (!isNotImplemented(e)) throw e;
    return { backups: mine([...sessionBackups]), source: "session" };
  }
}

/** A backup may only be restored onto the board it came from (same UID). */
export function canRestoreOnto(backup: Backup, boardUid: string | null | undefined): boolean {
  return !!boardUid && backup.boardUid === boardUid;
}

export interface RestoreResult {
  result: ApplyResult;
  ports: FcSerialPort[];
}

/**
 * Restore by id; the backend takes a safety backup first and refuses another firmware
 * line. Then re-read the ports so the wizard reflects the restored board.
 */
export async function restoreBackup(backup: Backup): Promise<RestoreResult> {
  const result = await api.config.restore(backup.id);
  if (result.errors.length > 0) throw uiError("CLI_ERROR", result.errors.map((e) => `${e.command}: ${e.response}`).join("\n"));
  if (!result.saved) throw uiError("CLI_ERROR", "restore was not saved");
  const ports = await api.config.readPorts();
  return { result, ports };
}
