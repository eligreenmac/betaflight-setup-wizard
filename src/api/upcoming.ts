// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * UI-side typed wrappers for contract items that were agreed but are NOT yet in
 * src/api/commands.ts / src/types/model.ts (another agent is adding them in a
 * `shared` PR). Nothing here changes the shared contract.
 *
 * TODO(shared): when the shared PR lands, move these entries into CommandMap /
 * model.ts, delete this file and point imports at `api.*`. Search for
 * "TODO(shared)" to find every call site.
 *
 * Until then every wrapper degrades gracefully: if the running backend does not
 * know a command, the call rejects with { code: "NOT_IMPLEMENTED" } and the UI
 * shows a disabled control with guidance instead of failing.
 */
import { invoke } from "@tauri-apps/api/core";
import { toUiError, uiError, type UiError } from "../errors/codes";

// ---- types (proposed shapes; confirm in the shared PR) -------------------------

/** `rc://channels` (DESIGN.md §4): raw values, usually 1000..2000, Betaflight channel order. */
export interface RcChannels {
  seq: number;
  timestampMs: number;
  channels: number[];
}
/** `rc://stopped` (DESIGN.md §4). */
export interface RcStopped {
  reason: "requested" | "disconnected" | "error";
  error: UiError | null;
}

// ---- motor test (DESIGN.md §4/§5) ------------------------------------------------

/** UI keepalive period while a motor test runs (backend watchdog default 500 ms). */
export const MOTOR_KEEPALIVE_MS = 200;
export const DEFAULT_MOTOR_WATCHDOG_MS = 500;
/** Backend default cap (15 % of the DShot range); over-cap requests are refused, not clamped. */
export const DEFAULT_MAX_TEST_THROTTLE = 1150;
export const HARD_MAX_TEST_THROTTLE = 1250;
/** What the wizard asks for when the user taps a motor: just enough to see direction. */
export const WIZARD_TEST_THROTTLE = 1100;

export interface MotorTestLimits {
  maxThrottle: number;
  /** Watchdog in ms (backend clamps to 100..2000). */
  watchdog: number;
}
/** TODO(shared): exact MotorOutputConfig fields (DESIGN.md names the type only). */
export interface MotorOutputConfig {
  motorCount?: number;
  stopValue?: number;
  protocol?: string;
  [k: string]: unknown;
}
export interface MotorTestStartArgs {
  port: string;
  /** 0-based Betaflight motor index; null = all motors. */
  motorIndex: number | null;
  /** Raw throttle value (stop value .. maxThrottle). */
  throttle: number;
  propsOffConfirmed: boolean;
  limits?: MotorTestLimits;
}
/** `motor-test://stopped` (DESIGN.md §4). */
export interface MotorTestStopped {
  reason: "requested" | "watchdog" | "armed" | "disconnected" | "error";
  error: UiError | null;
}


export const UPCOMING_EVENTS = {
  /** DESIGN.md §4 */
  motorTestStopped: "motor-test://stopped",
  rcChannels: "rc://channels",
  rcStopped: "rc://stopped",
} as const;

export interface UpcomingCommandMap {
  /** DESIGN.md §4: starts the session on first call, later calls update the throttle; feeds the watchdog. */
  motor_test_start: [MotorTestStartArgs, { outputConfig: MotorOutputConfig; limits: MotorTestLimits }];
  /** Every MOTOR_KEEPALIVE_MS while a test runs; MOTOR_TEST_NOT_ACTIVE once the watchdog fired. */
  motor_test_keepalive: [Record<string, never>, void];
  /** Stops all motors and hands the port back; DISCONNECTED when the port is gone. */
  motor_test_stop: [Record<string, never>, void];
  /** Emits `rc://channels` and `rc://stopped`. rateHz default 15, clamped 1..50. */
  rc_channels_stream_start: [{ port: string; rateHz?: number }, void];
  rc_channels_stream_stop: [Record<string, never>, void];
}
export type UpcomingCommand = keyof UpcomingCommandMap;
export type UpcomingBackend = <K extends UpcomingCommand>(cmd: K, args: UpcomingCommandMap[K][0]) => Promise<UpcomingCommandMap[K][1]>;

// ---- backend -----------------------------------------------------------------

/** Tauri reports unknown commands as a plain string; map that to NOT_IMPLEMENTED. */
export function toNotImplemented(cmd: string, e: unknown): UiError {
  if (typeof e === "string" && /not found|unknown command/i.test(e)) return uiError("NOT_IMPLEMENTED", `backend has no ${cmd} yet`);
  return toUiError(e);
}

const tauriBackend: UpcomingBackend = async (cmd, args) => {
  try {
    return (await invoke(cmd, args as Record<string, unknown>)) as never;
  } catch (e) {
    throw toNotImplemented(cmd, e);
  }
};

let backend: UpcomingBackend = tauriBackend;
export function setUpcomingBackend(b: UpcomingBackend): void {
  backend = b;
}
export function resetUpcomingBackend(): void {
  backend = tauriBackend;
}

/** TODO(shared): becomes `api.device.motorTest*` / `api.device.rcChannelsStream*`. */
const none = {} as Record<string, never>;
export const upcoming = {
  motorTestStart: (args: MotorTestStartArgs) => backend("motor_test_start", args),
  motorTestKeepalive: () => backend("motor_test_keepalive", none),
  motorTestStop: () => backend("motor_test_stop", none),
  rcChannelsStreamStart: (port: string, rateHz?: number) => backend("rc_channels_stream_start", rateHz === undefined ? { port } : { port, rateHz }),
  rcChannelsStreamStop: () => backend("rc_channels_stream_stop", none),
};
