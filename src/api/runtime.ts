// SPDX-License-Identifier: GPL-3.0-or-later
// Runtime detection lives in src/api/ because only this folder may touch @tauri-apps/*.
import { isTauri } from "@tauri-apps/api/core";

/** True inside the Tauri webview, false in a plain browser / Vitest. */
export function isTauriRuntime(): boolean {
  try {
    return isTauri();
  } catch {
    return false;
  }
}
