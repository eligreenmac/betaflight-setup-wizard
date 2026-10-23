// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * OSD step model. Positions and defaults come from the backend
 * (config_osd_default_layout, api/osdPending.ts); this file only keeps what the UI
 * adds: grouping, explanations (locale keys `osd.items.<name>`) and preview samples,
 * keyed by the firmware's `osd_<name>_pos` stem.
 *
 * TODO(shared): proposed DroneModel field (Config Engine Dev), not in src/types/model.ts yet:
 *   osd: { system: "ANALOG" | "HD", elements: [{ name, enabled, x?, y? }] } | null
 * null = keep the board's OSD. Only elements the user accepted (shown and kept) or
 * touched are sent; x/y only when the user moved the element (otherwise the engine
 * uses the default layout position for that system).
 */
import type { DroneModel, VtxKind } from "../types/model";
import type { OsdLayout, OsdSystem } from "../api/osdPending";

export type OsdGroup = "recommended" | "optional";
export interface OsdElementInfo {
  name: string;
  group: OsdGroup;
  /** Preview text (sample value). */
  sample: string;
  /** Only meaningful with a GPS. */
  gps?: boolean;
}

/** Elements the wizard offers, in display order. Anything else from the layout is not shown (and not sent). */
export const OSD_ELEMENTS: OsdElementInfo[] = [
  { name: "avg_cell_voltage", group: "recommended", sample: "4.12V" },
  { name: "vbat", group: "recommended", sample: "16.5V" },
  { name: "mah_drawn", group: "recommended", sample: "450MAH" },
  { name: "tim_2", group: "recommended", sample: "02:15" },
  { name: "link_quality", group: "recommended", sample: "LQ 100" },
  { name: "warnings", group: "recommended", sample: "LOW BATTERY" },
  { name: "craft_name", group: "recommended", sample: "MY QUAD" },
  { name: "gps_sats", group: "recommended", sample: "SAT 12", gps: true },
  { name: "home_dir", group: "recommended", sample: "^", gps: true },
  { name: "home_dist", group: "recommended", sample: "120M", gps: true },
  { name: "rssi", group: "optional", sample: "RSSI 99" },
  { name: "rssi_dbm", group: "optional", sample: "-70DBM" },
  { name: "throttle", group: "optional", sample: "THR 35" },
  { name: "current", group: "optional", sample: "12.3A" },
  { name: "flymode", group: "optional", sample: "ACRO" },
  { name: "altitude", group: "optional", sample: "35M" },
  { name: "gps_speed", group: "optional", sample: "42KM/H", gps: true },
  { name: "crosshairs", group: "optional", sample: "-+-" },
  { name: "tim_1", group: "optional", sample: "05:40" },
  { name: "disarmed", group: "optional", sample: "DISARMED" },
  { name: "esc_tmp", group: "optional", sample: "E 45C" },
];
export const OSD_GROUPS: OsdGroup[] = ["recommended", "optional"];

/** HD video systems that draw the OSD through MSP DisplayPort. */
const HD_SYSTEMS = new Set(["DJI", "WALKSNAIL", "HDZERO"]);

/** Video system from the VTX chosen in the wizard; null = unknown, ask the user. */
export function systemFromVtx(vtx: { kind: VtxKind; protocol?: string | null; digitalSystem?: string | null }): OsdSystem | null {
  if (vtx.kind === "ANALOG") return "ANALOG";
  if (vtx.kind === "DIGITAL" && (HD_SYSTEMS.has(vtx.digitalSystem ?? "") || vtx.protocol === "MSP")) return "HD";
  return null;
}

/** The system to use: the user's answer wins over the detection (they can correct it). */
export function effectiveSystem(model: DroneModel, s: OsdState): OsdSystem | null {
  return s.askedSystem ?? systemFromVtx(model.vtx);
}

/** Elements offered for this layout and model (in the layout, GPS ones only with a GPS). */
export function offeredElements(layout: OsdLayout | null, gps: boolean): OsdElementInfo[] {
  if (!layout) return [];
  const names = new Set(layout.elements.map((e) => e.name));
  return OSD_ELEMENTS.filter((e) => names.has(e.name) && (gps || !e.gps));
}

/** Recommended on/off = the backend's enabledByDefault for this system. */
export function recommendedEnabled(layout: OsdLayout, gps: boolean): Record<string, boolean> {
  const out: Record<string, boolean> = {};
  for (const e of offeredElements(layout, gps)) out[e.name] = layout.elements.find((l) => l.name === e.name)!.enabledByDefault;
  return out;
}

/** Keep an element (of `width` characters) inside the grid. */
export function clampPos(x: number, y: number, grid: { cols: number; rows: number }, width = 1): { x: number; y: number } {
  const w = Math.max(1, Math.min(width, grid.cols));
  return {
    x: Math.min(Math.max(0, Math.round(x)), grid.cols - w),
    y: Math.min(Math.max(0, Math.round(y)), grid.rows - 1),
  };
}

export interface OsdState {
  /** The user's answer when the VTX does not tell (null = not asked / not answered). */
  askedSystem: OsdSystem | null;
  /** keep = leave the board's OSD; choose = set it up here. */
  mode: "keep" | "choose" | null;
  layout: OsdLayout | null;
  /** System the layout was loaded for. */
  layoutSystem: OsdSystem | null;
  enabled: Record<string, boolean>;
  /** Positions the user moved (grid cells). */
  moved: Record<string, { x: number; y: number }>;
}
export const emptyOsdState = (): OsdState => ({ askedSystem: null, mode: null, layout: null, layoutSystem: null, enabled: {}, moved: {} });

export interface OsdChoice {
  system: OsdSystem;
  elements: { name: string; enabled: boolean; x?: number; y?: number }[];
}

/** The `osd` field to send, or null (= keep). */
export function osdChoice(s: OsdState, gps: boolean): OsdChoice | null {
  if (s.mode !== "choose" || !s.layout || !s.layoutSystem) return null;
  const elements = offeredElements(s.layout, gps).map((e) => {
    const out: OsdChoice["elements"][number] = { name: e.name, enabled: s.enabled[e.name] ?? false };
    const m = s.moved[e.name];
    if (m) Object.assign(out, clampPos(m.x, m.y, s.layout!.grid, e.sample.length));
    return out;
  });
  return { system: s.layoutSystem, elements };
}

export type ModelWithOsd = DroneModel & { osd?: OsdChoice };

/** Model for config_plan / config_verify: the shared model plus `osd` when chosen. */
export function withOsd(model: DroneModel, s: OsdState): ModelWithOsd {
  const out: ModelWithOsd = JSON.parse(JSON.stringify(model));
  const c = osdChoice(s, model.gps.enabled);
  if (c) out.osd = c;
  return out;
}

/** Where an element is drawn in the preview. */
export function previewPos(s: OsdState, name: string): { x: number; y: number } | null {
  const m = s.moved[name];
  if (m) return m;
  const l = s.layout?.elements.find((e) => e.name === name);
  return l ? { x: l.x, y: l.y } : null;
}
