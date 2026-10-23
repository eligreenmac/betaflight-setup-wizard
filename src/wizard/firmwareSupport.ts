// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Which Betaflight lines can the wizard set up? Asked of the config engine, never
 * hard-coded in the UI: `config_firmware_catalog(version)` is pure (no FC I/O) and fails
 * with UNSUPPORTED_FIRMWARE for a line the engine has no dialect for (e.g. 2026.12.x
 * while the backend supports 4.4 / 4.5 / 2025.12 / 2026.6). When Config adds a line, the UI
 * accepts it with no change here.
 *
 * Only a definite UNSUPPORTED_FIRMWARE marks a line unsupported; any other failure
 * (NOT_IMPLEMENTED on an older backend, a transient error) leaves it unknown, and
 * unknown never blocks. The oldest-version floor (MIN_SUPPORTED_FIRMWARE) stays separate:
 * it also covers non-Betaflight firmware and needs no backend.
 */
import { api } from "../api/commands";
import { toUiError } from "../errors/codes";
import type { WizardState } from "./state";

export type LineSupport = "supported" | "unsupported" | "unknown";

/** Firmware line of a version: its first two numeric parts ("2026.6.2" -> "2026.6", "4.5.1" -> "4.5"). */
export function firmwareLine(version: string | null | undefined): string {
  const parts = (version ?? "").trim().split(/[.-]/);
  const nums = parts.slice(0, 2).map((p) => (/^\d+$/.test(p) ? String(parseInt(p, 10)) : null));
  return nums.length === 2 && nums.every((n) => n !== null) ? nums.join(".") : "";
}

/** Known answer for this version's line ("unknown" until checked, or when the check could not tell). */
export function lineSupport(s: WizardState, version: string | null | undefined): LineSupport {
  const line = firmwareLine(version);
  return (line && s.ui.firmwareLines[line]) || "unknown";
}

type Ask = (version: string) => Promise<unknown>;
const askCatalog: Ask = (v) => api.config.firmwareCatalog(v);

/** Ask the config engine once per line; the answer is kept in `s.ui.firmwareLines`. */
export async function checkFirmwareSupport(s: WizardState, version: string | null | undefined, ask: Ask = askCatalog): Promise<LineSupport> {
  const line = firmwareLine(version);
  if (!line || !version) return "unknown";
  const known = s.ui.firmwareLines[line];
  if (known) return known;
  let answer: LineSupport;
  try {
    await ask(version);
    answer = "supported";
  } catch (e) {
    answer = toUiError(e).code === "UNSUPPORTED_FIRMWARE" ? "unsupported" : "unknown";
  }
  if (answer !== "unknown") s.ui.firmwareLines[line] = answer;
  return answer;
}

/** Check several versions (one call per distinct line). */
export async function checkFirmwareLines(s: WizardState, versions: readonly string[], ask: Ask = askCatalog): Promise<void> {
  const seen = new Map<string, string>();
  for (const v of versions) {
    const l = firmwareLine(v);
    if (l && !seen.has(l)) seen.set(l, v);
  }
  await Promise.all([...seen.values()].map((v) => checkFirmwareSupport(s, v, ask)));
}
