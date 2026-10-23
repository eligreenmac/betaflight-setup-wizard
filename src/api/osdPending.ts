// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Typed UI-side wrapper for Config's read-only `config_osd_default_layout`, which is in
 * shared PR #66 but not in this branch's CommandMap (src/api/commands.ts) yet. Calls go
 * through the same backend seam as `call()`, so the per-call demo fallback and the Demo
 * banner apply unchanged.
 *
 * TODO(shared): switch to api.config.osdDefaultLayout / OsdLayout from #66 when it lands,
 * then delete this file. Contract (#66, docs/ARCHITECTURE.md 2.1.1):
 *   config_osd_default_layout(videoSystem: "AUTO" | "PAL" | "NTSC" | "HD", firmwareVersion)
 *     -> OsdLayout { videoSystem, cols, rows, elements: [{ name, x, y, enabled }] }
 *   `name` is Betaflight's osd_items_e name without OSD_ (e.g. MAIN_BATT_VOLTAGE).
 *   Grids: PAL 30x16, NTSC 30x13, AUTO 30x13, HD 53x20 — but the UI never assumes a
 *   size: it draws whatever cols/rows come back. UNSUPPORTED_FIRMWARE for other lines.
 */
import { call, type CommandName } from "./commands";
import type { OsdLayout, OsdVideoSystem } from "../types/pendingModel";

export type { OsdLayout, OsdVideoSystem };
export const OSD_VIDEO_SYSTEMS: OsdVideoSystem[] = ["AUTO", "PAL", "NTSC", "HD"];
export const OSD_LAYOUT_COMMAND = "config_osd_default_layout";

const int = (v: unknown, min: number, max: number): number | null =>
  typeof v === "number" && Number.isInteger(v) && v >= min && v <= max ? v : null;

/** Tolerant parse: unknown keys ignored; elements without a valid name or outside the grid dropped. */
export function parseLayout(raw: unknown, requested: OsdVideoSystem): OsdLayout {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const videoSystem = OSD_VIDEO_SYSTEMS.includes(o.videoSystem as OsdVideoSystem) ? (o.videoSystem as OsdVideoSystem) : requested;
  // Fallback grid only if the backend omits it (it shouldn't): the smallest analog one.
  const cols = int(o.cols, 1, 63) ?? (videoSystem === "HD" ? 53 : 30);
  const rows = int(o.rows, 1, 31) ?? (videoSystem === "HD" ? 20 : 13);
  const seen = new Set<string>();
  const elements: OsdLayout["elements"] = [];
  for (const e of Array.isArray(o.elements) ? o.elements : []) {
    if (!e || typeof e !== "object") continue;
    const r = e as Record<string, unknown>;
    const name = typeof r.name === "string" && /^[A-Z0-9_]+$/.test(r.name) ? r.name : null;
    const x = int(r.x, 0, cols - 1);
    const y = int(r.y, 0, rows - 1);
    if (!name || x === null || y === null || seen.has(name)) continue;
    seen.add(name);
    elements.push({ name, x, y, enabled: r.enabled === true });
  }
  return { videoSystem, cols, rows, elements };
}

export const osdApi = {
  defaultLayout: async (videoSystem: OsdVideoSystem, firmwareVersion: string): Promise<OsdLayout> =>
    // The backend seam is untyped at runtime; CommandMap does not know this name yet.
    parseLayout(await call(OSD_LAYOUT_COMMAND as unknown as CommandName, { videoSystem, firmwareVersion } as never), videoSystem),
};
