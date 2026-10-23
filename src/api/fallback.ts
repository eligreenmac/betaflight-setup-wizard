// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Per-call demo fallback for preview builds.
 *
 * In the app, every command goes to the real backend first. If the backend does not
 * have it yet (a Phase 0 stub returning NOT_IMPLEMENTED, or Tauri's "command ... not
 * found" for a command that is not registered yet, e.g. the #38 device_* commands),
 * that one call is answered by the scenario mock instead and the command is recorded
 * in `demoState.simulated`. Any simulated call turns on the persistent "Demo mode"
 * banner (components/DemoBanner.vue). Real errors (PORT_BUSY, TIMEOUT, ...) are never
 * hidden behind the mock.
 *
 * `forceDemo()` sends every later call to the mock ("Try the demo" on the Connect step
 * when no board is plugged in), so every screen stays clickable to Done.
 *
 * SAFETY (never mix simulated and real data in a write): a write to the board (apply,
 * restore, flash, unprotect, raw CLI) must use the same source (real or mock) as the
 * board connection and as the data it writes (the plan, the firmware image, the DFU
 * device). A mocked config_plan is never sent to a real config_apply, and a real plan is
 * never "applied" by the mock. Mixed cases are refused with MIXED_BACKEND before
 * anything is sent (see writeBlock / WRITE_DEPENDENCIES).
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reactive } from "vue";
import { isNotImplemented, uiError } from "../errors/codes";
import { withLinkGate } from "./attitudeLink";
import { setBackend, type Backend, type CommandName } from "./commands";
import { setEventSource, type EventSource } from "./events";

export const demoState = reactive({
  /** Every call goes to the mock (browser demo, or "Try the demo"). */
  forced: false,
  /** Commands answered by the mock because the backend does not have them yet. */
  simulated: [] as string[],
  /** Where the last SUCCESSFUL call of each command was answered (write-safety guard). */
  sources: {} as Partial<Record<string, Source>>,
});

export type Source = "real" | "mock";

/** Calls that establish which board we talk to. The last successful one decides. */
const BOARD_COMMANDS: CommandName[] = ["device_connect", "device_identify"];

/**
 * Commands that write to the board, and the commands whose results they write.
 * Each must come from the same source as the board connection.
 */
export const WRITE_DEPENDENCIES: Partial<Record<string, string[]>> = {
  config_apply: ["config_plan", "config_read_ports"],
  config_rollback: ["config_backup"],
  config_restore: ["config_list_backups"],
  // TODO(shared #92): wizard/restore.ts. Restores the backup this update took.
  config_restore_after_flash: ["config_backup"],
  device_cli_exec: [],
  // flasher_required_options: never flash a real board with options a simulated backend chose.
  flasher_flash: ["flasher_fetch_firmware", "flasher_list_dfu_devices", "flasher_required_options"],
  flasher_remove_read_protection: ["flasher_list_dfu_devices"],
  // TODO(shared): pending device command (api/pending.ts). Calibration is stored on the board.
  device_acc_calibrate: [],
  // "Reboot board": never answered by the simulated board for a real one.
  device_reboot: [],
};

export const isWriteCommand = (cmd: string): boolean => cmd in WRITE_DEPENDENCIES;

function boardSource(): Source | undefined {
  for (const c of BOARD_COMMANDS) if (demoState.sources[c]) return demoState.sources[c];
  return undefined;
}

/** Sources of everything a write would rely on (board + its data), known so far. */
export function writeInputs(cmd: CommandName | string): { cmd: string; source: Source }[] {
  const out: { cmd: string; source: Source }[] = [];
  const b = boardSource();
  if (b) out.push({ cmd: "device_connect", source: b });
  for (const d of WRITE_DEPENDENCIES[cmd] ?? []) {
    const s = demoState.sources[d];
    if (s) out.push({ cmd: d, source: s });
  }
  return out;
}

/**
 * Would this write mix simulated and real data? Returns the commands of the minority
 * side, or null when everything known so far comes from one source. The UI calls it
 * before offering Apply; withFallback enforces it on every write.
 */
export function writeBlock(cmd: CommandName | string): { simulated: string[]; real: string[] } | null {
  if (demoState.forced) return null;
  const inputs = writeInputs(cmd);
  const simulated = inputs.filter((i) => i.source === "mock").map((i) => i.cmd);
  const real = inputs.filter((i) => i.source === "real").map((i) => i.cmd);
  return simulated.length && real.length ? { simulated, real } : null;
}

const mixed = (cmd: string, why: string) => uiError("MIXED_BACKEND", `${cmd} refused: ${why}`);

/**
 * The last answer to `cmd` was simulated while the board is real (outside forced demo).
 * Answers based on such data must never be written to the board (wizard/modelExt.ts).
 */
export function sourceMismatch(cmd: string): boolean {
  if (demoState.forced) return false;
  const s = demoState.sources[cmd];
  const b = boardSource();
  return !!s && !!b && s !== b;
}

export const isDemoMode = (): boolean => demoState.forced || demoState.simulated.length > 0;

export function forceDemo(): void {
  demoState.forced = true;
}

export function resetDemoState(): void {
  demoState.forced = false;
  demoState.simulated.splice(0);
  for (const k of Object.keys(demoState.sources)) delete demoState.sources[k];
}

function markSimulated(cmd: string): void {
  if (!demoState.simulated.includes(cmd)) demoState.simulated.push(cmd);
}

type AnyBackend = (cmd: string, args: never) => Promise<unknown>;

/**
 * Real first; on NOT_IMPLEMENTED / unknown command, the mock answers that call.
 * Writes follow the write-safety rule above instead of falling back blindly.
 */
export function withFallback(real: Backend, mock: Backend): Backend {
  const r = real as unknown as AnyBackend;
  const m = mock as unknown as AnyBackend;
  const viaMock = async (cmd: string, args: never) => {
    markSimulated(cmd);
    const v = await m(cmd, args);
    demoState.sources[cmd] = "mock";
    return v;
  };
  return (async (cmd: string, args: never) => {
    if (demoState.forced) return viaMock(cmd, args);
    if (isWriteCommand(cmd)) {
      const block = writeBlock(cmd as CommandName);
      if (block) throw mixed(cmd, `simulated ${block.simulated.join(", ")} with real ${block.real.join(", ")}`);
      const inputs = writeInputs(cmd as CommandName);
      // Prepared on the mock: the write stays on the mock, never on a real board.
      if (inputs.length && inputs.every((i) => i.source === "mock")) return viaMock(cmd, args);
      try {
        const v = await r(cmd, args);
        demoState.sources[cmd] = "real";
        return v;
      } catch (e) {
        if (!isNotImplemented(e)) throw e;
        // Real board / real data but no real write yet: never pretend it was written.
        if (inputs.length) throw mixed(cmd, "the backend cannot write yet; the board and its data are real");
        return viaMock(cmd, args);
      }
    }
    try {
      const v = await r(cmd, args);
      demoState.sources[cmd] = "real";
      return v;
    } catch (e) {
      if (!isNotImplemented(e)) throw e;
      return viaMock(cmd, args);
    }
  }) as unknown as Backend;
}

/** Listen on both sources: real backend events and the mock's simulated ones. */
export function mergeEventSources(a: EventSource, b: EventSource): EventSource {
  return async (name, handler) => {
    const offs = await Promise.all([a(name, handler).catch(() => () => undefined), b(name, handler)]);
    return () => offs.forEach((off) => off());
  };
}

export interface FallbackTargets {
  backend: Backend;
  events: EventSource;
}

const tauriTargets = (): FallbackTargets => ({
  backend: ((cmd: string, args: unknown) => tauriInvoke(cmd, args as Record<string, unknown>)) as Backend,
  events: (name, handler) => listen(name, (e) => handler(e.payload as never)),
});

/** Wire real-with-mock-fallback into every backend seam (app runtime). */
export function installFallback(mock: FallbackTargets, real: FallbackTargets = tauriTargets()): void {
  setBackend(withLinkGate(withFallback(real.backend, mock.backend)));
  setEventSource(mergeEventSources(real.events, mock.events));
}
