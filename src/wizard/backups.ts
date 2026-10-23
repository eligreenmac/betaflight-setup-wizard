// SPDX-License-Identifier: GPL-3.0-or-later
/** Backup listing and restore (config_rollback). */
import { api } from "../api/commands";
import { upcoming } from "../api/upcoming";
import { isNotImplemented, uiError } from "../errors/codes";
import type { ApplyResult, Backup, FcSerialPort } from "../types/model";

export interface BackupList {
  backups: Backup[];
  /** "backend" = config_list_backups; "session" = only backups made in this session (TODO(shared)). */
  source: "backend" | "session";
}

const newestFirst = (a: Backup, b: Backup) => b.createdAt.localeCompare(a.createdAt);

export async function listBackups(sessionBackups: Backup[]): Promise<BackupList> {
  try {
    return { backups: [...(await upcoming.configListBackups())].sort(newestFirst), source: "backend" };
  } catch (e) {
    if (!isNotImplemented(e)) throw e;
    return { backups: [...sessionBackups].sort(newestFirst), source: "session" };
  }
}

/** A backup may only be restored onto the board it came from. */
export function canRestoreOnto(backup: Backup, boardName: string | null | undefined): boolean {
  return !!boardName && backup.boardName === boardName;
}

export interface RestoreResult {
  result: ApplyResult;
  ports: FcSerialPort[];
}

/** Restore, then re-read the ports so the wizard reflects the restored board. */
export async function restoreBackup(backup: Backup): Promise<RestoreResult> {
  const result = await api.config.rollback(backup);
  if (result.errors.length > 0) throw uiError("CLI_ERROR", result.errors.map((e) => `${e.command}: ${e.response}`).join("\n"));
  const ports = await api.config.readPorts();
  return { result, ports };
}
