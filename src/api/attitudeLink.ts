// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Sharing the board connection with the live attitude stream (Device #94, ARCHITECTURE §4.3).
 *
 * While the stream runs, device_identify, device_sensors and device_get_attitude still work:
 * the stream pauses for them. Everything else that talks to the board (device_acc_calibrate,
 * device_reboot, device_probe_board, device_cli_exec, Config's link commands, ...) gets
 * PORT_BUSY, so the stream has to be stopped first.
 *
 * - stopAttitudeStream(): stop the stream and wait until it has really stopped (the stop call
 *   answered OK or `device://attitude-stopped` arrived). A PORT_BUSY stop (a shared call holds
 *   the port) is retried for a few seconds; after that it rejects with LIVE_VIEW_BUSY.
 * - withLinkGate(backend): a command that needs the port to itself waits for a stop that is
 *   still in flight (e.g. the Calibration step stopping the stream as it is left) before it is
 *   sent, instead of racing it and getting PORT_BUSY.
 */
import { api, EVENTS, type Backend } from "./commands";
import { toUiError, uiError } from "../errors/codes";
import { onEvent } from "./events";

/** Commands that run next to the attitude stream (and the stream's own commands, which must never wait on themselves). */
export const SHARES_PORT_WITH_ATTITUDE: readonly string[] = [
  "device_identify",
  "device_sensors",
  "device_get_attitude",
  "device_attitude_stream_start",
  "device_attitude_stream_stop",
];

/**
 * Retrying the stop. While a shared call (identify / sensors / get_attitude) holds the port,
 * device_attitude_stream_stop itself can answer PORT_BUSY: retry with a short backoff, for at
 * most `totalMs`. Mutable for tests only.
 */
export const STOP_RETRY = { totalMs: 4000, backoffMs: [50, 100, 200, 400] as number[] };

let inFlight: Promise<void> | null = null;

/**
 * Stop the attitude stream and resolve only once it has really stopped: the stop call answered
 * OK, or `device://attitude-stopped` arrived. PORT_BUSY from the stop is retried (STOP_RETRY);
 * any other error from the stop (not connected, no stream, a backend without the command)
 * leaves nothing to stop. Rejects with LIVE_VIEW_BUSY when the stream could not be stopped in
 * time: the caller must then NOT send its command.
 */
export function stopAttitudeStream(): Promise<void> {
  const p = (async () => {
    let stopped = false;
    let wake: () => void = () => undefined;
    const off = await onEvent(EVENTS.attitudeStopped, () => {
      stopped = true;
      wake();
    }).catch(() => null);
    const woken = (ms: number) =>
      new Promise<void>((r) => {
        const t = setTimeout(r, ms);
        wake = () => {
          clearTimeout(t);
          r();
        };
      });
    const deadline = Date.now() + STOP_RETRY.totalMs;
    let attempt = 0;
    try {
      while (!stopped) {
        const left = deadline - Date.now();
        if (left <= 0) break;
        // The stop call, bounded by the time left (and ended early by the stopped event).
        const outcome = await Promise.race([
          api.device.attitudeStreamStop().then(
            () => "ok" as const,
            (e) => (toUiError(e).code === "PORT_BUSY" ? ("busy" as const) : ("gone" as const)),
          ),
          woken(left).then(() => (stopped ? ("ok" as const) : ("timeout" as const))),
        ]);
        if (outcome === "ok" || outcome === "gone") return;
        if (outcome === "timeout") break;
        const pause = Math.min(STOP_RETRY.backoffMs[Math.min(attempt++, STOP_RETRY.backoffMs.length - 1)]!, Math.max(0, deadline - Date.now()));
        await woken(pause);
      }
      if (stopped) return;
      throw uiError("LIVE_VIEW_BUSY", `device_attitude_stream_stop: the stream did not stop within ${STOP_RETRY.totalMs} ms (PORT_BUSY)`);
    } finally {
      off?.();
    }
  })();
  inFlight = p;
  const clear = () => {
    if (inFlight === p) inFlight = null;
  };
  p.then(clear, clear);
  return p;
}

/** Resolves when no attitude stop is in flight. */
export const attitudeStopping = (): Promise<void> => (inFlight ? inFlight.catch(() => undefined) : Promise.resolve());

/** Hold back commands that need the port to themselves while an attitude stop is in flight. */
export function withLinkGate(backend: Backend): Backend {
  return (async (cmd: string, args: never) => {
    if (inFlight && !SHARES_PORT_WITH_ATTITUDE.includes(cmd)) await inFlight.catch(() => undefined);
    return (backend as unknown as (c: string, a: never) => Promise<unknown>)(cmd, args);
  }) as unknown as Backend;
}
