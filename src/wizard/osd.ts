// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * OSD step model. Positions and defaults come from Config
 * (config_osd_default_layout(videoSystem, firmwareVersion), api/osdLayout.ts) and the
 * element list per firmware from config_firmware_catalog (api/catalog.ts); this file
 * only keeps what the UI adds: grouping, explanations (locale keys `osd.items.<NAME>`)
 * and preview samples, keyed by Betaflight's osd_items_e name without OSD_.
 *
 * Sent as the #65 `osd` field (types/model.ts):
 *   osd: { videoSystem: AUTO | PAL | NTSC | HD, elements: [{ name, enabled, x?, y? }] } | null
 * null/absent = keep the board's OSD. Only elements the wizard offers are sent (elements
 * not listed keep the FC's state); x/y only when the user moved the element (otherwise
 * Config uses its default layout position for that video system).
 */
import type { DroneModel, FcInfo, VtxKind } from "../types/model";
import type { Osd, OsdLayout, OsdVideoSystem } from "../types/model";

export type OsdGroup = "recommended" | "optional";
export interface OsdElementInfo {
  name: string;
  group: OsdGroup;
  /** Preview text (sample value). */
  sample: string;
  /** Only meaningful with a GPS. */
  gps?: boolean;
}

/** Elements the wizard offers, in display order. Anything else in the layout is not shown (and not sent). */
export const OSD_ELEMENTS: OsdElementInfo[] = [
  { name: "AVG_CELL_VOLTAGE", group: "recommended", sample: "4.12V" },
  { name: "MAIN_BATT_VOLTAGE", group: "recommended", sample: "16.5V" },
  { name: "MAH_DRAWN", group: "recommended", sample: "450MAH" },
  { name: "ITEM_TIMER_2", group: "recommended", sample: "02:15" },
  { name: "LINK_QUALITY", group: "recommended", sample: "LQ 100" },
  { name: "WARNINGS", group: "recommended", sample: "LOW BATTERY" },
  { name: "CRAFT_NAME", group: "recommended", sample: "MY QUAD" },
  { name: "GPS_SATS", group: "recommended", sample: "SAT 12", gps: true },
  { name: "HOME_DIR", group: "recommended", sample: "^", gps: true },
  { name: "HOME_DIST", group: "recommended", sample: "120M", gps: true },
  { name: "RSSI_VALUE", group: "optional", sample: "RSSI 99" },
  { name: "RSSI_DBM_VALUE", group: "optional", sample: "-70DBM" },
  { name: "THROTTLE_POS", group: "optional", sample: "THR 35" },
  { name: "CURRENT_DRAW", group: "optional", sample: "12.3A" },
  { name: "FLYMODE", group: "optional", sample: "ACRO" },
  { name: "ALTITUDE", group: "optional", sample: "35M" },
  { name: "GPS_SPEED", group: "optional", sample: "42KM/H", gps: true },
  { name: "CROSSHAIRS", group: "optional", sample: "-+-" },
  { name: "ITEM_TIMER_1", group: "optional", sample: "05:40" },
  { name: "DISARMED", group: "optional", sample: "DISARMED" },
  { name: "ESC_TMP", group: "optional", sample: "E 45C" },
];
export const OSD_GROUPS: OsdGroup[] = ["recommended", "optional"];
export const ANALOG_SYSTEMS: OsdVideoSystem[] = ["AUTO", "PAL", "NTSC"];
export const isAnalog = (s: OsdVideoSystem | null): boolean => !!s && s !== "HD";

/** HD video systems that draw the OSD through MSP DisplayPort. */
const HD_SYSTEMS = new Set(["DJI", "WALKSNAIL", "HDZERO"]);

/**
 * Video system from the VTX chosen in the wizard; null = unknown, ask the user.
 * Analog defaults to AUTO (the board follows the camera, PAL or NTSC).
 */
export function systemFromVtx(vtx: { kind: VtxKind; protocol?: string | null; digitalSystem?: string | null }): OsdVideoSystem | null {
  if (vtx.kind === "ANALOG") return "AUTO";
  if (vtx.kind === "DIGITAL" && (HD_SYSTEMS.has(vtx.digitalSystem ?? "") || vtx.protocol === "MSP")) return "HD";
  return null;
}

/** The build option that means the firmware has the HD (MSP DisplayPort canvas) OSD. */
const HD_OSD_OPTION = "USE_OSD_HD";
/**
 * Build options that show the OSD was chosen at build time, so a missing USE_OSD_HD is
 * meaningful. USE_MSP_DISPLAYPORT alone is NOT HD: it is also used without an HD OSD
 * (e.g. for an analog/SD OSD over DisplayPort).
 */
const OSD_OPTIONS = ["USE_OSD", "USE_OSD_SD", "USE_MSP_DISPLAYPORT"];

/**
 * The FC fields the HD gate reads. `hdOsdSupported` is Device's shared field
 * (`FcInfo.hdOsdSupported: boolean | null`): true = HD / MSP DisplayPort OSD compiled in,
 * false = confirmed absent, null = unknown. Optional here so older backends / fixtures
 * that omit it still type-check (they fall back to the build options).
 */
export type HdOsdFc = Pick<FcInfo, "buildOptions"> & Partial<Pick<FcInfo, "hdOsdSupported">>;

/**
 * Fallback when `hdOsdSupported` is null: USE_OSD_HD = true; any other OSD option
 * (USE_OSD, USE_OSD_SD, USE_MSP_DISPLAYPORT) without it = false; no OSD option at all
 * (including an empty list, as on 4.4 / unknown) = null.
 */
function hdFromBuildOptions(fc: Pick<FcInfo, "buildOptions"> | null): boolean | null {
  const opts = (Array.isArray(fc?.buildOptions) ? fc!.buildOptions : []).map((o) => String(o).trim().toUpperCase());
  if (opts.includes(HD_OSD_OPTION)) return true;
  if (opts.some((o) => OSD_OPTIONS.includes(o))) return false;
  return null;
}

/**
 * Does this board's firmware support an HD OSD (MSP DisplayPort)?
 * true = yes; false = confirmed absent (HD disabled, with the reason); null = unknown
 * (HD allowed, with a note that it could not be confirmed).
 * Reads `fc.hdOsdSupported` tolerantly (only a real boolean counts); when it is missing,
 * null or malformed, falls back to the build-options heuristic.
 */
export function hdOsdSupport(fc: HdOsdFc | null): boolean | null {
  const v = (fc as { hdOsdSupported?: unknown } | null)?.hdOsdSupported;
  if (typeof v === "boolean") return v;
  return hdFromBuildOptions(fc);
}

/**
 * HD support for the OSD step: false also when config_plan already said this firmware
 * has no MSP DisplayPort (Review -> "Back to OSD settings"), otherwise hdOsdSupport.
 */
export function hdSupport(fc: HdOsdFc | null, s: Pick<OsdState, "hdRejected">): boolean | null {
  return s.hdRejected ? false : hdOsdSupport(fc);
}

/**
 * The system to use: the user's answer wins over the detection (they can correct it).
 * A detected HD that this firmware can't do is not used: the user is asked instead.
 */
export function effectiveSystem(model: DroneModel, s: OsdState, fc: HdOsdFc | null = null): OsdVideoSystem | null {
  if (s.askedSystem) return s.askedSystem;
  const d = systemFromVtx(model.vtx);
  return d === "HD" && hdSupport(fc, s) === false ? null : d;
}

/** Digital VTX with an analog OSD: allowed (some setups do it on purpose), but probably wrong. */
export function analogOnDigital(model: DroneModel, system: OsdVideoSystem | null): boolean {
  return model.vtx.kind === "DIGITAL" && isAnalog(system);
}

/**
 * Elements offered for this layout and model: in the layout, in the firmware's catalog
 * (when known), and GPS ones only with a GPS.
 */
export function offeredElements(layout: OsdLayout | null, gps: boolean, catalog: string[] = []): OsdElementInfo[] {
  if (!layout) return [];
  const names = new Set(layout.elements.map((e) => e.name));
  const inCatalog = catalog.length ? new Set(catalog.map((c) => c.toUpperCase())) : null;
  return OSD_ELEMENTS.filter((e) => names.has(e.name) && (!inCatalog || inCatalog.has(e.name)) && (gps || !e.gps));
}

/** Recommended on/off = the layout's `enabled` for this video system. */
export function recommendedEnabled(layout: OsdLayout, gps: boolean, catalog: string[] = []): Record<string, boolean> {
  const out: Record<string, boolean> = {};
  for (const e of offeredElements(layout, gps, catalog)) out[e.name] = layout.elements.find((l) => l.name === e.name)!.enabled;
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
  /** The user's choice (null = not chosen: the detected system is used). */
  askedSystem: OsdVideoSystem | null;
  /** keep = leave the board's OSD; choose = set it up here. */
  mode: "keep" | "choose" | null;
  layout: OsdLayout | null;
  /** System and firmware the layout was loaded for ("HD@4.5.2"). */
  layoutKey: string | null;
  /** The firmware's OSD element names (catalog); [] = unknown, no filter. */
  catalog: string[];
  enabled: Record<string, boolean>;
  /** Positions the user moved (grid cells). */
  moved: Record<string, { x: number; y: number }>;
  /** config_plan refused HD for this board (no MSP DisplayPort): HD is off until the board changes. */
  hdRejected: boolean;
}
export const emptyOsdState = (): OsdState => ({ askedSystem: null, mode: null, layout: null, layoutKey: null, catalog: [], enabled: {}, moved: {}, hdRejected: false });

/**
 * config_plan's error for an HD OSD on firmware without MSP DisplayPort (config::generate,
 * #67): AppError::Config "HD OSD needs MSP DisplayPort, which this firmware build does not
 * have" (code CONFIG -> CONFIG_UNSUPPORTED in the UI).
 */
export function isHdOsdPlanError(e: { code: string; message: string }): boolean {
  return (e.code === "CONFIG_UNSUPPORTED" || e.code === "CONFIG") && /HD OSD|MSP DisplayPort/i.test(e.message);
}
export const layoutKey = (system: OsdVideoSystem, firmwareVersion: string): string => `${system}@${firmwareVersion}`;

/** The #65 `osd` field to send, or null (= keep). */
export function osdChoice(s: OsdState, gps: boolean): Osd | null {
  if (s.mode !== "choose" || !s.layout) return null;
  const grid = { cols: s.layout.cols, rows: s.layout.rows };
  const elements = offeredElements(s.layout, gps, s.catalog).map((e) => {
    const out: Osd["elements"][number] = { name: e.name, enabled: s.enabled[e.name] ?? false };
    const m = s.moved[e.name];
    if (m) Object.assign(out, clampPos(m.x, m.y, grid, e.sample.length));
    return out;
  });
  return { videoSystem: s.layout.videoSystem, elements };
}

export type ModelWithOsd = DroneModel & { osd?: Osd | null };

/** Model for config_plan / config_verify: the shared model plus `osd` when chosen. */
export function withOsd<M extends DroneModel>(model: M, s: OsdState): M & { osd?: Osd | null } {
  const out: M & { osd?: Osd | null } = JSON.parse(JSON.stringify(model));
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
