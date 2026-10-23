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
 */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { reactive } from "vue";
import { isNotImplemented } from "../errors/codes";
import { setBackend, type Backend } from "./commands";
import { setDeviceBackend, type DeviceBackend } from "./device";
import { setEventSource, type EventSource } from "./events";

export const demoState = reactive({
  /** Every call goes to the mock (browser demo, or "Try the demo"). */
  forced: false,
  /** Commands answered by the mock because the backend does not have them yet. */
  simulated: [] as string[],
});

export const isDemoMode = (): boolean => demoState.forced || demoState.simulated.length > 0;

export function forceDemo(): void {
  demoState.forced = true;
}

export function resetDemoState(): void {
  demoState.forced = false;
  demoState.simulated.splice(0);
}

function markSimulated(cmd: string): void {
  if (!demoState.simulated.includes(cmd)) demoState.simulated.push(cmd);
}

type AnyBackend = (cmd: string, args: never) => Promise<unknown>;

/** Real first; on NOT_IMPLEMENTED / unknown command, the mock answers that call. */
export function withFallback<B extends Backend | DeviceBackend>(real: B, mock: B): B {
  const r = real as unknown as AnyBackend;
  const m = mock as unknown as AnyBackend;
  return (async (cmd: string, args: never) => {
    if (demoState.forced) {
      markSimulated(cmd);
      return m(cmd, args);
    }
    try {
      return await r(cmd, args);
    } catch (e) {
      if (!isNotImplemented(e)) throw e;
      markSimulated(cmd);
      return m(cmd, args);
    }
  }) as unknown as B;
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
  deviceBackend: DeviceBackend;
  events: EventSource;
}

const tauriTargets = (): FallbackTargets => ({
  backend: ((cmd: string, args: unknown) => tauriInvoke(cmd, args as Record<string, unknown>)) as Backend,
  deviceBackend: ((cmd: string, args: unknown) => tauriInvoke(cmd, args as Record<string, unknown>)) as DeviceBackend,
  events: (name, handler) => listen(name, (e) => handler(e.payload as never)),
});

/** Wire real-with-mock-fallback into every backend seam (app runtime). */
export function installFallback(mock: FallbackTargets, real: FallbackTargets = tauriTargets()): void {
  setBackend(withFallback(real.backend, mock.backend));
  setDeviceBackend(withFallback(real.deviceBackend, mock.deviceBackend));
  setEventSource(mergeEventSources(real.events, mock.events));
}
