// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Config's read-only `config_firmware_catalog(firmwareVersion)` (shared #66,
 * `api.config.firmwareCatalog`, type `FirmwareCatalog` in types/model.ts): what this
 * firmware line has (beeper conditions, DShot beacon events, OSD elements, mode names).
 * The UI uses it instead of hard-coded lists. Always answered by the real backend (config
 * engine, #67). The mock (mocks/scenarios.ts mockFirmwareCatalog) answers only in demo
 * mode, or for an older backend that returns NOT_IMPLEMENTED (api/fallback.ts).
 *
 * The result is parsed tolerantly: `{name}` objects are accepted, duplicates and unknown
 * keys dropped, so a newer backend never breaks the steps.
 */
import { reactive } from "vue";
import { api } from "./commands";
import { toUiError, type UiError } from "../errors/codes";
import type { BeeperName, DshotBeaconCondition, FirmwareCatalog } from "../types/model";

export type { FirmwareCatalog };

function names(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  const out: string[] = [];
  for (const x of v) {
    const n = typeof x === "string" ? x : x && typeof x === "object" && typeof (x as { name?: unknown }).name === "string" ? (x as { name: string }).name : null;
    if (n && !out.includes(n)) out.push(n);
  }
  return out;
}

/** Unknown beeper / beacon names are kept: the steps only show the names they know. */
export function parseCatalog(raw: unknown): FirmwareCatalog {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  return {
    beepers: names(o.beepers) as BeeperName[],
    beaconEvents: names(o.beaconEvents) as DshotBeaconCondition[],
    osdElements: names(o.osdElements),
    modes: names(o.modes),
  };
}

export const catalogApi = {
  get: async (firmwareVersion: string): Promise<FirmwareCatalog> => parseCatalog(await api.config.firmwareCatalog(firmwareVersion)),
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
