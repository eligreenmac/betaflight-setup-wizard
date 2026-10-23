// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Typed UI-side wrapper for Config's read-only `config_firmware_catalog(firmwareVersion)`
 * (shared PR #66; mocked in demo mode). What this firmware line has:
 * beeper conditions, DShot beacon events, OSD elements and modes. The UI uses it instead
 * of hard-coded lists.
 *
 * TODO(shared): use api.config.firmwareCatalog / FirmwareCatalog from #66 when it lands.
 * Shape as in #66 (parsed tolerantly: `{name}` objects are accepted, unknown keys ignored):
 *   { beepers: BeeperName[], beaconEvents: DshotBeaconCondition[], osdElements: string[], modes: string[] }
 */
import { reactive } from "vue";
import { call, type CommandName } from "./commands";
import { toUiError, type UiError } from "../errors/codes";

export const CATALOG_COMMAND = "config_firmware_catalog";

export interface FirmwareCatalog {
  /** Beeper CLI names (#65 BeeperName spelling, e.g. CRASH_FLIP). */
  beepers: string[];
  /** DShot beacon events (`beacon` CLI names, e.g. RX_LOST, RX_SET). */
  beaconEvents: string[];
  /** OSD element names (osd_items_e without OSD_). */
  osdElements: string[];
  /** Mode names (config::modes stable names, e.g. "AIR MODE"). */
  modes: string[];
}

function names(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  const out: string[] = [];
  for (const x of v) {
    const n = typeof x === "string" ? x : x && typeof x === "object" && typeof (x as { name?: unknown }).name === "string" ? (x as { name: string }).name : null;
    if (n && !out.includes(n)) out.push(n);
  }
  return out;
}

export function parseCatalog(raw: unknown): FirmwareCatalog {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  return { beepers: names(o.beepers), beaconEvents: names(o.beaconEvents), osdElements: names(o.osdElements), modes: names(o.modes) };
}

export const catalogApi = {
  get: async (firmwareVersion: string): Promise<FirmwareCatalog> =>
    parseCatalog(await call(CATALOG_COMMAND as unknown as CommandName, { firmwareVersion } as never)),
};

/** Loaded catalog for the connected firmware (shared by the steps that need it). */
export const catalogState = reactive({
  version: null as string | null,
  catalog: null as FirmwareCatalog | null,
  loading: false,
  error: null as UiError | null,
});

/** Load (once per firmware version) and cache. Errors are kept in catalogState.error. */
export async function loadCatalog(firmwareVersion: string | null | undefined): Promise<FirmwareCatalog | null> {
  const v = firmwareVersion ?? "";
  if (catalogState.catalog && catalogState.version === v) return catalogState.catalog;
  catalogState.loading = true;
  catalogState.error = null;
  try {
    const c = await catalogApi.get(v);
    catalogState.catalog = c;
    catalogState.version = v;
    return c;
  } catch (e) {
    catalogState.error = toUiError(e);
    return null;
  } finally {
    catalogState.loading = false;
  }
}

export function resetCatalog(): void {
  Object.assign(catalogState, { version: null, catalog: null, loading: false, error: null });
}

const squash = (s: string) => s.toUpperCase().replace(/[^A-Z0-9]/g, "");
/** Is `name` in the list (spaces / underscores ignored)? */
export const catalogHas = (list: string[], name: string): boolean => list.some((x) => squash(x) === squash(name));
