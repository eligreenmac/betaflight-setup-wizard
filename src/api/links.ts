// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Help links and the backups folder.
 *
 * In the desktop app these go through tauri-plugin-opener, which needs the scoped
 * capability from the separate `shared` PR (branch feat/opener-capability). This
 * branch does NOT depend on it: we call the plugin's IPC commands directly and, if
 * the plugin or the capability is missing (command not found / not allowed), the
 * caller gets `{ opened: false }` and shows the address as a normal link to copy.
 * In the browser demo a normal new-tab link is used.
 */
import { invoke, isTauri } from "@tauri-apps/api/core";
import { appDataDir, join } from "@tauri-apps/api/path";

/** Must match opener:allow-open-url in the shared capability. */
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

type Invoker = (cmd: string, args: Record<string, unknown>) => Promise<unknown>;
let invoker: Invoker = (cmd, args) => invoke(cmd, args);
let inApp: () => boolean = () => isTauri();
/** Test seam. */
export function setLinkRuntime(rt: { invoke?: Invoker; inApp?: () => boolean } | null): void {
  invoker = rt?.invoke ?? ((cmd, args) => invoke(cmd, args));
  inApp = rt?.inApp ?? (() => isTauri());
}

export async function openExternal(url: string): Promise<OpenResult> {
  if (!isAllowedUrl(url)) return { opened: false, target: url };
  if (!inApp()) {
    const w = typeof window !== "undefined" ? window.open(url, "_blank", "noopener,noreferrer") : null;
    return { opened: w !== null, target: url };
  }
  try {
    await invoker("plugin:opener|open_url", { url });
    return { opened: true, target: url };
  } catch {
    // TODO(shared): plugin/capability not registered yet (feat/opener-capability).
    return { opened: false, target: url };
  }
}

/** %APPDATA%\<app>\backups\<board-uid>\ (Tauri appDataDir = bundle identifier folder). */
export async function backupsFolder(boardUid: string | null): Promise<string> {
  const base = await join(await appDataDir(), "backups");
  return boardUid ? join(base, boardUid) : base;
}

/** Open the board's backups folder in Explorer; `{opened:false}` = show the path instead. */
export async function openBackupsFolder(boardUid: string | null): Promise<OpenResult> {
  if (!inApp()) return { opened: false, target: `%APPDATA%\\io.github.eligreenmac.bfsw\\backups\\${boardUid ?? ""}` };
  const dir = await backupsFolder(boardUid);
  try {
    await invoker("plugin:opener|open_path", { path: dir });
    return { opened: true, target: dir };
  } catch {
    return { opened: false, target: dir };
  }
}
