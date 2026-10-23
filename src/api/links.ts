// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Help links and the backups folder, via @tauri-apps/plugin-opener (scoped
 * capability in src-tauri/capabilities/default.json). If opening fails (e.g. a path
 * outside the scope), the caller gets `{ opened: false }` and shows the address as
 * text to copy. In the browser demo a normal new-tab link is used.
 */
import { isTauri } from "@tauri-apps/api/core";
import { join } from "@tauri-apps/api/path";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { api } from "./commands";

/** Must match opener:allow-open-url in src-tauri/capabilities/default.json. */
export const ALLOWED_URL_PREFIXES = [
  "https://betaflight.com/",
  "https://github.com/betaflight/",
  "https://github.com/eligreenmac/betaflight-setup-wizard/",
] as const;

export function isAllowedUrl(url: string): boolean {
  return ALLOWED_URL_PREFIXES.some((p) => url.startsWith(p));
}

export interface OpenResult {
  opened: boolean;
  /** What to show the user when `opened` is false. */
  target: string;
}

interface LinkRuntime {
  inApp: () => boolean;
  openUrl: (url: string) => Promise<void>;
  openPath: (path: string) => Promise<void>;
  backupsDir: () => Promise<string>;
  join: (...parts: string[]) => Promise<string>;
}
const defaults: LinkRuntime = {
  inApp: () => isTauri(),
  openUrl: (url) => openUrl(url),
  openPath: (path) => openPath(path),
  backupsDir: () => api.config.backupsDir(),
  join: (...parts) => join(...parts),
};
let rt: LinkRuntime = defaults;
/** Test seam. */
export function setLinkRuntime(over: Partial<LinkRuntime> | null): void {
  rt = { ...defaults, ...(over ?? {}) };
}

export async function openExternal(url: string): Promise<OpenResult> {
  if (!isAllowedUrl(url)) return { opened: false, target: url };
  if (!rt.inApp()) {
    const w = typeof window !== "undefined" ? window.open(url, "_blank", "noopener,noreferrer") : null;
    return { opened: w !== null, target: url };
  }
  try {
    await rt.openUrl(url);
    return { opened: true, target: url };
  } catch {
    return { opened: false, target: url };
  }
}

/** `<config_backups_dir>/<boardUid>` (ARCHITECTURE §5.1), or the root without a board. */
export async function backupsFolder(boardUid: string | null): Promise<string> {
  const base = await rt.backupsDir();
  return boardUid ? rt.join(base, boardUid) : base;
}

/** Open the board's backups folder in Explorer; `{opened:false}` = show the path instead. */
export async function openBackupsFolder(boardUid: string | null): Promise<OpenResult> {
  if (!rt.inApp()) return { opened: false, target: `%APPDATA%\\io.github.eligreenmac.bfsw\\backups\\${boardUid ?? ""}` };
  let dir: string;
  try {
    dir = await backupsFolder(boardUid);
  } catch {
    return { opened: false, target: `%APPDATA%\\io.github.eligreenmac.bfsw\\backups\\${boardUid ?? ""}` };
  }
  try {
    await rt.openPath(dir);
    return { opened: true, target: dir };
  } catch {
    return { opened: false, target: dir };
  }
}
