// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Motor test session, UI side (DESIGN.md §4/§5).
 *
 * Betaflight has NO firmware-side MSP motor timeout: if USB drops mid-test the last
 * value stays applied. So the UI:
 *   - refuses to start without BOTH acknowledgements ("props removed" AND
 *     "battery disconnected, USB power only"), without calling the backend;
 *   - sends motor_test_keepalive every 200 ms while a test runs (the backend
 *     watchdog stops motors after 500 ms without one);
 *   - calls motor_test_stop on: window blur, page hide / beforeunload, app close,
 *     step or route navigation, component unmount, and any IPC error.
 * The backend's own watchdog, arming re-check and Drop-stop remain the last line.
 */
import { reactive, watch } from "vue";
import { onEvent, type Unlisten } from "../api/events";
import { MOTOR_KEEPALIVE_MS, DEFAULT_MAX_TEST_THROTTLE, DEFAULT_MOTOR_WATCHDOG_MS, UPCOMING_EVENTS, WIZARD_TEST_THROTTLE, upcoming, type MotorTestStartArgs } from "../api/upcoming";
import { toUiError, uiError, type UiError } from "../errors/codes";

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
  start: (a) => upcoming.motorTestStart(a),
  keepalive: () => upcoming.motorTestKeepalive(),
  stop: () => upcoming.motorTestStop(),
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
        port: opts.port,
        motorIndex: motor,
        throttle: opts.throttle ?? WIZARD_TEST_THROTTLE,
        propsOffConfirmed: true,
        limits: { maxThrottle: DEFAULT_MAX_TEST_THROTTLE, watchdog: DEFAULT_MOTOR_WATCHDOG_MS },
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
    if (!wasActive) return;
    this.state.lastStop = trigger;
    if (trigger === "backend") return; // backend already stopped (watchdog/armed/disconnect)
    try {
      await this.deps.stop();
    } catch (e) {
      // Never silent (DESIGN.md §4): DISCONNECTED here means the port is gone.
      this.state.error ??= toUiError(e);
    }
  }

  /** Backend reported `motor-test://stopped` (watchdog, armed, disconnected, error). */
  backendStopped(reason: string, error: UiError | null): void {
    void this.stop("backend", error ?? (reason === "requested" ? undefined : uiError(reason === "armed" ? "BOARD_ARMED" : reason === "watchdog" ? "MOTOR_TEST_NOT_ACTIVE" : "DISCONNECTED", `motor test stopped: ${reason}`)));
  }
}

/** The app-wide session (one motor test at a time). */
export const motorTest = new MotorTestController();

export interface StopTriggerSources {
  /** window (blur, pagehide, beforeunload, popstate, hashchange). */
  target?: Pick<EventTarget, "addEventListener" | "removeEventListener">;
  /** Wizard step id (or route); any change stops the motors. */
  navigation?: () => unknown;
  /** Subscribe to backend `motor-test://stopped` events. */
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
    void onEvent(UPCOMING_EVENTS.motorTestStopped, (p) => ctl.backendStopped(p.reason, p.error))
      .then((u) => (disposed ? u() : (off = u)))
      .catch(() => undefined);
    disposers.push(() => {
      disposed = true;
      off?.();
    });
  }
  return () => disposers.forEach((d) => d());
}
