// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * App-close hook for safety cleanups (stop motors).
 *
 * Tauri's onCloseRequested destroys the window itself after our handler, which needs
 * the `core:window:allow-destroy` permission; without it a JS close listener would
 * keep the window from closing. That permission is a capability (shared) change, so
 * the hook is OFF on this branch: motors are still stopped on blur / pagehide /
 * beforeunload in the webview, and by the backend (watchdog + Drop of the motor
 * session on exit, DESIGN.md §5).
 * TODO(shared): add core:window:allow-destroy (or stop motors in Rust on
 * WindowEvent::CloseRequested) and set CLOSE_HOOK_ENABLED = true.
 */
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Unlisten } from "./events";

export const CLOSE_HOOK_ENABLED = false;

export async function onAppCloseRequested(handler: () => Promise<void>): Promise<Unlisten> {
  if (!CLOSE_HOOK_ENABLED || !isTauri()) return () => undefined;
  return getCurrentWindow().onCloseRequested(async () => {
    await handler();
  });
}
