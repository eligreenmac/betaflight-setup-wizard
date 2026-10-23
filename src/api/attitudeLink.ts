// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Sharing the board connection with the live attitude stream (Device #94, ARCHITECTURE §4.3).
 *
 * While the stream runs, device_identify, device_sensors and device_get_attitude still work:
 * the stream pauses for them. Everything else that talks to the board (device_acc_calibrate,
 * device_reboot, device_probe_board, device_cli_exec, Config's link commands, ...) gets
 * PORT_BUSY, so the stream has to be stopped first.
 *
 * - stopAttitudeStream(): stop the stream and wait until the port is free: until the stop call
 *   settles or `device://attitude-stopped` arrives, whichever comes first (never longer than
 *   STOP_WAIT_MS, so a lost board cannot hang the UI).
 * - withLinkGate(backend): a command that needs the port to itself waits for a stop that is
 *   still in flight (e.g. the Calibration step stopping the stream as it is left) before it is
 *   sent, instead of racing it and getting PORT_BUSY.
 */
import { api, EVENTS, type Backend } from "./commands";
import { onEvent, type Unlisten } from "./events";

/** Commands that run next to the attitude stream (and the stream's own commands, which must never wait on themselves). */
export const SHARES_PORT_WITH_ATTITUDE: readonly string[] = [
  "device_identify",
  "device_sensors",
  "device_get_attitude",
  "device_attitude_stream_start",
  "device_attitude_stream_stop",
];

/** Upper bound for waiting on the stop (the backend answers in milliseconds). */
export const STOP_WAIT_MS = 3000;

let inFlight: Promise<void> | null = null;

/** Stop the attitude stream and resolve once the port is free. Never rejects. */
export function stopAttitudeStream(): Promise<void> {
  const p = new Promise<void>((resolve) => {
    let done = false;
    let off: Unlisten | null = null;
    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      off?.();
      resolve();
    };
    const timer = setTimeout(finish, STOP_WAIT_MS);
    onEvent(EVENTS.attitudeStopped, finish).then(
      (o) => (done ? o() : (off = o)),
      () => undefined,
    );
    // A failed stop (not connected, no stream) leaves nothing to wait for either.
    api.device.attitudeStreamStop().then(finish, finish);
  });
  inFlight = p;
  void p.then(() => {
    if (inFlight === p) inFlight = null;
  });
  return p;
}

/** Resolves when no attitude stop is in flight. */
export const attitudeStopping = (): Promise<void> => inFlight ?? Promise.resolve();

/** Hold back commands that need the port to themselves while an attitude stop is in flight. */
export function withLinkGate(backend: Backend): Backend {
  return (async (cmd: string, args: never) => {
    if (inFlight && !SHARES_PORT_WITH_ATTITUDE.includes(cmd)) await inFlight;
    return (backend as unknown as (c: string, a: never) => Promise<unknown>)(cmd, args);
  }) as unknown as Backend;
}
