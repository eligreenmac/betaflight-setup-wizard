// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Typed UI-side wrapper for the read-only OSD layout command proposed by Config Engine
 * Dev; it is not in the shared CommandMap (src/api/commands.ts) yet. Calls go through
 * the same backend seam as `call()`, so the per-call demo fallback and the Demo banner
 * apply unchanged.
 *
 * TODO(shared): move to CommandMap when Config's `shared` PR lands, then delete this file.
 * Contract as agreed (2026-10-23):
 *   config_osd_default_layout(system: "ANALOG" | "HD")
 *     -> { grid: { cols, rows }, elements: [{ name, x, y, enabledByDefault }] }
 *   `name` is the firmware's `osd_<name>_pos` stem (e.g. "vbat", "link_quality").
 *   The grid depends on the board (analog PAL / NTSC / AUTO = 30x16 or 30x13, HD canvas),
 *   so the UI never assumes a size. Positions outside the grid are a CONFIG error.
 */
import { call, type CommandName } from "./commands";

export type OsdSystem = "ANALOG" | "HD";
export interface OsdLayoutElement {
  name: string;
  x: number;
  y: number;
  enabledByDefault: boolean;
}
export interface OsdLayout {
  grid: { cols: number; rows: number };
  elements: OsdLayoutElement[];
}

export const OSD_LAYOUT_COMMAND = "config_osd_default_layout";

const int = (v: unknown, min: number, max: number): number | null =>
  typeof v === "number" && Number.isInteger(v) && v >= min && v <= max ? v : null;

/** Tolerant parse: unknown keys ignored; elements without a name or outside the grid dropped. */
export function parseLayout(raw: unknown): OsdLayout {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const g = o.grid && typeof o.grid === "object" ? (o.grid as Record<string, unknown>) : {};
  const cols = int(g.cols, 1, 255) ?? 30;
  const rows = int(g.rows, 1, 255) ?? 16;
  const seen = new Set<string>();
  const elements: OsdLayoutElement[] = [];
  for (const e of Array.isArray(o.elements) ? o.elements : []) {
    if (!e || typeof e !== "object") continue;
    const r = e as Record<string, unknown>;
    const name = typeof r.name === "string" && /^[a-z0-9_]+$/.test(r.name) ? r.name : null;
    const x = int(r.x, 0, cols - 1);
    const y = int(r.y, 0, rows - 1);
    if (!name || x === null || y === null || seen.has(name)) continue;
    seen.add(name);
    elements.push({ name, x, y, enabledByDefault: r.enabledByDefault === true });
  }
  return { grid: { cols, rows }, elements };
}

export const osdApi = {
  defaultLayout: async (system: OsdSystem): Promise<OsdLayout> =>
    // The backend seam is untyped at runtime; CommandMap does not know this name yet.
    parseLayout(await call(OSD_LAYOUT_COMMAND as unknown as CommandName, { system } as never)),
};
