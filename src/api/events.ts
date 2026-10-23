// SPDX-License-Identifier: GPL-3.0-or-later
// Typed event subscription. In the app this is Tauri's `listen`; tests and browser
// demo mode swap in an in-memory source with setEventSource().

import { listen } from "@tauri-apps/api/event";
import type { FlashProgress } from "../types/model";
import { EVENTS } from "./commands";
import { UPCOMING_EVENTS, type ApplyProgress, type RcChannels } from "./upcoming";

export interface EventMap {
  [EVENTS.flashProgress]: FlashProgress;
  [UPCOMING_EVENTS.applyProgress]: ApplyProgress;
  [UPCOMING_EVENTS.rcChannels]: RcChannels;
}
export type EventName = keyof EventMap;
export type Unlisten = () => void;
export type EventSource = <K extends EventName>(name: K, handler: (payload: EventMap[K]) => void) => Promise<Unlisten>;

const tauriSource: EventSource = (name, handler) => listen(name, (e) => handler(e.payload as never));

let source: EventSource = tauriSource;

export function setEventSource(s: EventSource): void {
  source = s;
}
export function resetEventSource(): void {
  source = tauriSource;
}

export function onEvent<K extends EventName>(name: K, handler: (payload: EventMap[K]) => void): Promise<Unlisten> {
  return source(name, handler);
}

/** Minimal in-memory emitter usable as an EventSource (mocks, tests). */
export function createEmitter() {
  const handlers = new Map<string, Set<(p: unknown) => void>>();
  const on: EventSource = async (name, handler) => {
    const set = handlers.get(name) ?? new Set();
    set.add(handler as (p: unknown) => void);
    handlers.set(name, set);
    return () => set.delete(handler as (p: unknown) => void);
  };
  function emit<K extends EventName>(name: K, payload: EventMap[K]): void {
    for (const h of handlers.get(name) ?? []) h(payload);
  }
  const listenerCount = (name: EventName) => handlers.get(name)?.size ?? 0;
  return { on, emit, listenerCount };
}
export type Emitter = ReturnType<typeof createEmitter>;
