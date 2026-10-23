// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Firmware build options from the wizard's answers (Device #74/#75), and the mid-wizard
 * "Update the firmware to add this" flow.
 *
 * - requirementsFor: the FirmwareRequirements runFirmwareUpdate passes to
 *   flasher_required_options. Only what is chosen (null otherwise); nothing chosen = null,
 *   so a first flash keeps the board's own options.
 * - Mid-wizard re-flash: a step (OSD, Receiver) sets `ui.reflash` {returnTo, requirements}
 *   after the user confirmed, and jumps to the Firmware step. That step flashes through
 *   the normal flow (backup first, reboot to DFU, flash, reconnect) with those extra
 *   requirements, then returns to `returnTo`. All answers stay in the app.
 * - Safety (Chief of Staff): only before any config_apply in this session, and never when
 *   flasher_flash would mix simulated and real data (api/fallback.ts guard).
 * - missingOptions: which of the options an answer needs are not in the board's build.
 *   Unknown (no build options, backend can't say, simulated answer for a real board) = null,
 *   which never blocks.
 */
import { api } from "../api/commands";
import { sourceMismatch, writeBlock } from "../api/fallback";
import type { FcInfo, FirmwareRequirements } from "../types/model";
import { effectiveSystem, systemFromVtx } from "./osd";
import type { WizardState } from "./state";

export interface ReflashRequest {
  /** Step id to come back to after the update. */
  returnTo: string;
  /** What this step needs on top of the wizard's other answers. */
  requirements: FirmwareRequirements;
}

/** The receiver and OSD answers, as far as they are chosen (null = not chosen). */
export function requirementsFor(s: WizardState): FirmwareRequirements | null {
  const r = s.model.receiver;
  const receiver = r.kind !== "NONE" && r.protocol ? { kind: r.kind, protocol: r.protocol } : null;
  const system = s.osd.mode === "choose" ? effectiveSystem(s.model, s.osd, s.fc) : null;
  const osd = system ? { videoSystem: system } : null;
  const extra = s.ui.reflash?.requirements ?? {};
  const out: FirmwareRequirements = { receiver: extra.receiver ?? receiver, osd: extra.osd ?? osd };
  return out.receiver || out.osd ? out : null;
}

export type ReflashBlock = "applied" | "mixed" | "noBoard";

/** Why the mid-wizard update can't be offered now (null = it can). */
export function reflashBlock(s: WizardState): ReflashBlock | null {
  if (!s.fc) return "noBoard";
  if (s.ui.configApplied) return "applied";
  if (writeBlock("flasher_flash")) return "mixed";
  return null;
}

/** The user confirmed: remember where to come back to and what to add. */
export function requestReflash(s: WizardState, req: ReflashRequest): boolean {
  if (reflashBlock(s)) return false;
  s.ui.reflash = { returnTo: req.returnTo, requirements: { ...req.requirements } };
  return true;
}

/** After a successful update (or Cancel): forget the request; returns the step to go back to. */
export function finishReflash(s: WizardState, flashed = true): string | null {
  const r = s.ui.reflash;
  s.ui.reflash = null;
  if (!r) return null;
  // The user wanted HD and the board can do it now: keep that answer.
  if (flashed && r.requirements.osd?.videoSystem === "HD" && s.fc?.hdOsdSupported !== false) {
    const detectedHd = systemFromVtx(s.model.vtx) === "HD";
    s.osd.askedSystem = detectedHd && !s.osd.askedSystem ? null : "HD";
  }
  return r.returnTo;
}

/**
 * Options `requirements` needs that the board's build lacks. null = can't tell (no build
 * options, e.g. 4.4; the backend has no flasher_required_options; or its answer was
 * simulated while the board is real). Never throws.
 */
export async function missingOptions(fc: FcInfo | null, requirements: FirmwareRequirements): Promise<string[] | null> {
  if (!fc || !Array.isArray(fc.buildOptions) || fc.buildOptions.length === 0) return null;
  try {
    const need = await api.flasher.requiredOptions(requirements, null);
    if (sourceMismatch("flasher_required_options")) return null;
    const have = new Set(fc.buildOptions.map((o) => o.toUpperCase()));
    return need.filter((o) => !have.has(o.toUpperCase()));
  } catch {
    return null;
  }
}
