// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Motor test session, UI side (DESIGN.md §4/§5).
 *
 * Betaflight has NO firmware-side MSP motor timeout: if USB drops mid-test the last
 * value stays applied. So the UI:
 *   - refuses to start without BOTH acknowledgements ("props removed" AND
 *     "battery disconnected, USB power only"), without calling the backend;
 *   - sends device_motor_test_keepalive every 200 ms while a test runs (the backend
 *     watchdog stops motors after `limits.watchdogMs`, default 500, without one);
 *   - calls device_motor_test_stop on: window blur, page hide / beforeunload, step or
 *     route navigation, component unmount, any IPC error, and after every
 *     `device://motor-test-stopped` event.
 * Window close / app exit are handled by the backend (emergency stop in the Tauri run
 * loop, ARCHITECTURE §4.2), as are its watchdog and arming re-check.
 * The connection manager owns the port, so no port is passed (ARCHITECTURE §4.1).
 */
import { reactive, watch } from "vue";
import { api, EVENTS } from "../api/commands";
import type { MotorTestLimits, MotorTestStopReason } from "../types/model";
import { onEvent, type Unlisten } from "../api/events";
import { toUiError, uiError, type ErrorCode, type UiError } from "../errors/codes";

/** UI keepalive period while a motor test runs. */
export const MOTOR_KEEPALIVE_MS = 200;
/** Backend watchdog default (clamped 100..2000). */
export const DEFAULT_MOTOR_WATCHDOG_MS = 500;
/** Backend default cap; over-cap requests are refused, not clamped. Hard cap 1250. */
export const DEFAULT_MAX_TEST_THROTTLE = 1150;
export const HARD_MAX_TEST_THROTTLE = 1250;
/** What the wizard asks for when the user taps a motor: just enough to see direction. */
export const WIZARD_TEST_THROTTLE = 1100;
export const WIZARD_MOTOR_LIMITS: MotorTestLimits = { maxThrottle: DEFAULT_MAX_TEST_THROTTLE, watchdogMs: DEFAULT_MOTOR_WATCHDOG_MS };

export interface MotorTestStartArgs {
  /** 0-based Betaflight motor index; null = all motors. */
  motorIndex: number | null;
  throttle: number;
  propsOffConfirmed: boolean;
  limits: MotorTestLimits | null;
}

/** Error shown for a backend stop reason (null = normal stop, no error). */
export const STOP_REASON_ERROR: Record<MotorTestStopReason, ErrorCode | null> = {
  REQUESTED: null,
  APP_CLOSING: null,
  WATCHDOG: "MOTOR_TEST_NOT_ACTIVE",
  ARMED: "BOARD_ARMED",
  DISCONNECTED: "DISCONNECTED",
  ERROR: "UNKNOWN",
};

export interface MotorAcks {
  propsRemoved: boolean;
  batteryDisconnected: boolean;
}
export const acksComplete = (a: MotorAcks): boolean => a.propsRemoved === true && a.batteryDisconnected === true;

export type StopTrigger = "requested" | "blur" | "pagehide" | "beforeunload" | "navigation" | "unmount" | "ipc-error" | "backend" | "acks-withdrawn";

export interface MotorTestDeps {
  start: (a: MotorTestStartArgs) => Promise<unknown>;
  keepalive: () => Promise<void>;
  stop: () => Promise<void>;
  setInterval: (fn: () => void, ms: number) => unknown;
  clearInterval: (h: unknown) => void;
}

export const defaultMotorDeps: MotorTestDeps = {
  start: (a) => api.device.motorTestStart(a.motorIndex, a.throttle, a.propsOffConfirmed, a.limits),
  keepalive: () => api.device.motorTestKeepalive(),
  stop: () => api.device.motorTestStop(),
  setInterval: (fn, ms) => globalThis.setInterval(fn, ms),
  clearInterval: (h) => globalThis.clearInterval(h as ReturnType<typeof setInterval>),
};

export interface MotorTestState {
  /** A start was requested and no stop has happened since (includes the in-flight start). */
  active: boolean;
  running: boolean;
  motor: number | null;
  lastStop: StopTrigger | null;
  error: UiError | null;
}

export class MotorTestController {
  readonly state: MotorTestState = reactive({ active: false, running: false, motor: null, lastStop: null, error: null });
  private timer: unknown = null;
  /** Bumped on every stop so a start that resolves after a stop never resumes keepalives. */
  private generation = 0;

  constructor(private readonly deps: MotorTestDeps = defaultMotorDeps) {}

  async start(motor: number, opts: { port: string | null; acks: MotorAcks; throttle?: number }): Promise<void> {
    if (!acksComplete(opts.acks)) {
      const e = uiError("MOTOR_TEST_REFUSED", "confirm that the propellers are removed and the battery is disconnected");
      this.state.error = e;
      throw e;
    }
    if (!opts.port) {
      const e = uiError("DISCONNECTED", "no flight controller connected");
      this.state.error = e;
      throw e;
    }
    const gen = this.generation;
    this.state.active = true;
    this.state.error = null;
    try {
      await this.deps.start({
        motorIndex: motor,
        throttle: opts.throttle ?? WIZARD_TEST_THROTTLE,
        propsOffConfirmed: true,
        limits: WIZARD_MOTOR_LIMITS,
      });
    } catch (e) {
      const ue = toUiError(e);
      await this.stop("ipc-error", ue);
      throw ue;
    }
    if (gen !== this.generation) {
      // Stopped while the start was in flight: the backend may have started the motor
      // AFTER our stop arrived, so send exactly one more stop (unless a newer start owns it).
      if (!this.state.active && !this.state.running) {
        try {
          await this.deps.stop();
        } catch (e) {
          this.state.error ??= toUiError(e);
        }
      }
      return;
    }
    this.state.running = true;
    this.state.motor = motor;
    if (this.timer === null) this.timer = this.deps.setInterval(() => void this.tick(), MOTOR_KEEPALIVE_MS);
  }

  private async tick(): Promise<void> {
    if (!this.state.running) return;
    try {
      await this.deps.keepalive();
    } catch (e) {
      await this.stop("ipc-error", toUiError(e));
    }
  }

  /** Stop everything. Safe to call any time; only talks to the backend if a test is (or may be) running. */
  async stop(trigger: StopTrigger, error?: UiError): Promise<void> {
    const wasActive = this.state.active || this.state.running || this.timer !== null;
    this.generation++;
    if (this.timer !== null) {
      this.deps.clearInterval(this.timer);
      this.timer = null;
    }
    this.state.active = false;
    this.state.running = false;
    this.state.motor = null;
    if (error) this.state.error = error;
    // A backend stop event is always followed by device_motor_test_stop (idempotent),
    // so the backend's session is released even if the UI thought nothing was running.
    if (!wasActive && trigger !== "backend") return;
    if (wasActive) this.state.lastStop = trigger;
    try {
      await this.deps.stop();
    } catch (e) {
      // Never silent (DESIGN.md §4): DISCONNECTED here means the port is gone.
      this.state.error ??= toUiError(e);
    }
  }

  /** Backend reported `device://motor-test-stopped`; returns the stop it triggered. */
  backendStopped(reason: MotorTestStopReason, error: unknown): Promise<void> {
    const active = this.state.active || this.state.running || this.timer !== null;
    // REQUESTED while idle is the echo of our own stop: answering it could loop.
    if (reason === "REQUESTED" && !active) return Promise.resolve();
    const code = STOP_REASON_ERROR[reason];
    const e = error ? toUiError(error) : code ? uiError(code, `motor test stopped: ${reason}`) : undefined;
    return this.stop("backend", e);
  }
}

/** The app-wide session (one motor test at a time). */
export const motorTest = new MotorTestController();

export interface StopTriggerSources {
  /** window (blur, pagehide, beforeunload, popstate, hashchange). */
  target?: Pick<EventTarget, "addEventListener" | "removeEventListener">;
  /** Wizard step id (or route); any change stops the motors. */
  navigation?: () => unknown;
  /** Subscribe to backend `device://motor-test-stopped` events. */
  backendEvents?: boolean;
}

/** Wire every stop trigger to `ctl`. Returns a disposer. */
export function bindMotorStopTriggers(ctl: MotorTestController, src: StopTriggerSources): () => void {
  const disposers: (() => void)[] = [];
  if (src.target) {
    const t = src.target;
    const map: [string, StopTrigger][] = [
      ["blur", "blur"],
      ["pagehide", "pagehide"],
      ["beforeunload", "beforeunload"],
      ["popstate", "navigation"],
      ["hashchange", "navigation"],
    ];
    for (const [ev, trig] of map) {
      const h = () => void ctl.stop(trig);
      t.addEventListener(ev, h);
      disposers.push(() => t.removeEventListener(ev, h));
    }
  }
  if (src.navigation) disposers.push(watch(src.navigation, () => void ctl.stop("navigation"), { flush: "sync" }));
  if (src.backendEvents) {
    let off: Unlisten | null = null;
    let disposed = false;
    void onEvent(EVENTS.motorTestStopped, (p) => void ctl.backendStopped(p.reason, p.error))
      .then((u) => (disposed ? u() : (off = u)))
      .catch(() => undefined);
    disposers.push(() => {
      disposed = true;
      off?.();
    });
  }
  return () => disposers.forEach((d) => d());
}
