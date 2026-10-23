// SPDX-License-Identifier: GPL-3.0-or-later
// Open help links in the system browser via tauri-plugin-opener. The allow-list
// below MUST match src-tauri/capabilities/default.json (opener:allow-open-url).
import { appDataDir, join } from "@tauri-apps/api/path";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { isTauri } from "@tauri-apps/api/core";

export const ALLOWED_URL_PREFIXES = [
  "https://betaflight.com/",
  "https://github.com/betaflight/",
  "https://github.com/eligreenmac/betaflight-setup-wizard/",
] as const;

export function isAllowedUrl(url: string): boolean {
  return ALLOWED_URL_PREFIXES.some((p) => url.startsWith(p));
}

export async function openExternal(url: string): Promise<void> {
  if (!isAllowedUrl(url)) throw { code: "INVALID_ARGUMENT", message: `link not allowed: ${url}` };
  if (isTauri()) await openUrl(url);
  else window.open(url, "_blank", "noopener,noreferrer");
}

/**
 * Backups live in %APPDATA%\<app>\backups\<board-uid>\ (Tauri appDataDir, i.e. the
 * bundle identifier folder). The capability only allows paths under $APPDATA/backups.
 * TODO(config): confirm the folder layout the config engine writes.
 */
export async function backupsFolder(boardUid: string | null): Promise<string> {
  const base = await join(await appDataDir(), "backups");
  return boardUid ? join(base, boardUid) : base;
}

/** Open the board's backups folder in Explorer. Rejects NOT_IMPLEMENTED outside the app. */
export async function openBackupsFolder(boardUid: string | null): Promise<string> {
  if (!isTauri()) throw { code: "NOT_IMPLEMENTED", message: "opening folders needs the desktop app" };
  const dir = await backupsFolder(boardUid);
  await openPath(dir);
  return dir;
}
