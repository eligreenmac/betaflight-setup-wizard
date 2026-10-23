// SPDX-License-Identifier: GPL-3.0-or-later
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick, reactive } from "vue";
import { createEmitter } from "../src/api/events";
import { setEventSource, resetEventSource } from "../src/api/events";
import { uiError } from "../src/errors/codes";
import { installMock, uninstallMock } from "../src/mocks/install";
import { bindMotorStopTriggers, defaultMotorDeps, MOTOR_KEEPALIVE_MS, MotorTestController, type MotorTestDeps } from "../src/wizard/motorTest";

const ACKS = { propsRemoved: true, batteryDisconnected: true };
const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

function fakeDeps() {
  const log: string[] = [];
  const deps: MotorTestDeps & { log: string[]; failKeepalive: boolean } = {
    log,
    failKeepalive: false,
    start: vi.fn(async (a) => void log.push(`start ${a.motorIndex} ${a.throttle} ${a.propsOffConfirmed} ${a.limits?.maxThrottle}/${a.limits?.watchdogMs}`)),
    keepalive: vi.fn(async () => {
      log.push("keepalive");
      if (deps.failKeepalive) throw uiError("DISCONNECTED", "port gone");
    }),
    stop: vi.fn(async () => void log.push("stop")),
    setInterval: (fn, ms) => setInterval(fn, ms),
    clearInterval: (h) => clearInterval(h as ReturnType<typeof setInterval>),
  };
  return deps;
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  uninstallMock();
});

describe("motor test safety gate", () => {
  it("refuses without BOTH acknowledgements and never calls the backend", async () => {
    const d = fakeDeps();
    const c = new MotorTestController(d);
    await expect(c.start(0, { port: "COM5", acks: { propsRemoved: true, batteryDisconnected: false } })).rejects.toMatchObject({ code: "MOTOR_TEST_REFUSED" });
    await expect(c.start(0, { port: "COM5", acks: { propsRemoved: false, batteryDisconnected: true } })).rejects.toMatchObject({ code: "MOTOR_TEST_REFUSED" });
    await expect(c.start(0, { port: null, acks: ACKS })).rejects.toMatchObject({ code: "DISCONNECTED" });
    expect(d.start).not.toHaveBeenCalled();
    await c.start(1, { port: "COM5", acks: ACKS });
    expect(d.log[0]).toBe("start 1 1100 true 1150/500");
  });
});

describe("keepalive timer", () => {
  it("sends device_motor_test_keepalive every 200 ms while running and stops sending after stop", async () => {
    expect(MOTOR_KEEPALIVE_MS).toBe(200);
    const d = fakeDeps();
    const c = new MotorTestController(d);
    await c.start(0, { port: "COM5", acks: ACKS });
    await vi.advanceTimersByTimeAsync(199);
    expect(d.keepalive).toHaveBeenCalledTimes(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(d.keepalive).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(800);
    expect(d.keepalive).toHaveBeenCalledTimes(5);
    // switching motor keeps a single timer
    await c.start(2, { port: "COM5", acks: ACKS });
    await vi.advanceTimersByTimeAsync(200);
    expect(d.keepalive).toHaveBeenCalledTimes(6);
    await c.stop("requested");
    await vi.advanceTimersByTimeAsync(1000);
    expect(d.keepalive).toHaveBeenCalledTimes(6);
    expect(d.log.at(-1)).toBe("stop");
    expect(c.state.running).toBe(false);
  });

  it("a keepalive IPC error stops the motors and surfaces the error", async () => {
    const d = fakeDeps();
    const c = new MotorTestController(d);
    await c.start(0, { port: "COM5", acks: ACKS });
    d.failKeepalive = true;
    await vi.advanceTimersByTimeAsync(200);
    await flush();
    expect(d.stop).toHaveBeenCalledTimes(1);
    expect(c.state.lastStop).toBe("ipc-error");
    expect(c.state.error).toMatchObject({ code: "DISCONNECTED" });
    await vi.advanceTimersByTimeAsync(1000);
    expect(d.keepalive).toHaveBeenCalledTimes(1);
  });

  it("a start IPC error sends a stop anyway", async () => {
    const d = fakeDeps();
    d.start = vi.fn(async () => {
      throw uiError("BOARD_ARMED", "armed");
    });
    const c = new MotorTestController(d);
    await expect(c.start(0, { port: "COM5", acks: ACKS })).rejects.toMatchObject({ code: "BOARD_ARMED" });
    expect(d.stop).toHaveBeenCalledTimes(1);
    expect(c.state.lastStop).toBe("ipc-error");
  });

  it("a stop during an in-flight start never resumes keepalives", async () => {
    const d = fakeDeps();
    let release!: () => void;
    d.start = vi.fn(() => new Promise<void>((r) => (release = r)));
    const c = new MotorTestController(d);
    const p = c.start(0, { port: "COM5", acks: ACKS });
    await c.stop("blur");
    expect(d.stop).toHaveBeenCalledTimes(1);
    release();
    await p;
    expect(d.stop).toHaveBeenCalledTimes(2); // one extra stop for the late start
    await vi.advanceTimersByTimeAsync(1000);
    expect(d.keepalive).not.toHaveBeenCalled();
    expect(c.state.running).toBe(false);
  });

  it("stop is a no-op (no backend call) when nothing runs", async () => {
    const d = fakeDeps();
    const c = new MotorTestController(d);
    await c.stop("blur");
    expect(d.stop).not.toHaveBeenCalled();
  });
});

describe("stop triggers", () => {
  async function running() {
    const d = fakeDeps();
    const c = new MotorTestController(d);
    await c.start(3, { port: "COM5", acks: ACKS });
    return { d, c };
  }

  for (const ev of ["blur", "pagehide", "beforeunload", "popstate", "hashchange"]) {
    it(`window ${ev} stops the motors`, async () => {
      const { d, c } = await running();
      const target = new EventTarget();
      const off = bindMotorStopTriggers(c, { target });
      target.dispatchEvent(new Event(ev));
      await flush();
      expect(d.stop).toHaveBeenCalledTimes(1);
      expect(c.state.running).toBe(false);
      off();
      // after dispose, events no longer reach the controller
      await c.start(3, { port: "COM5", acks: ACKS });
      target.dispatchEvent(new Event(ev));
      await flush();
      expect(c.state.running).toBe(true);
    });
  }

  it("step navigation stops the motors", async () => {
    const { d, c } = await running();
    const nav = reactive({ currentId: "esc" });
    const off = bindMotorStopTriggers(c, { navigation: () => nav.currentId });
    nav.currentId = "switches";
    await nextTick();
    await flush();
    expect(d.stop).toHaveBeenCalledTimes(1);
    expect(c.state.lastStop).toBe("navigation");
    off();
  });

  it("a start that resolves after a stop gets exactly one extra stop; a timely start gets none", async () => {
    const d = fakeDeps();
    const order: string[] = [];
    let release!: () => void;
    d.start = vi.fn(() => new Promise<void>((r) => (release = () => (order.push("start resolved"), r()))));
    d.stop = vi.fn(async () => void order.push("stop"));
    const c = new MotorTestController(d);
    const p = c.start(2, { port: "COM5", acks: ACKS });
    await c.stop("navigation");
    await c.stop("unmount"); // already stopped: no backend call
    release();
    await p;
    await flush();
    expect(order).toEqual(["stop", "start resolved", "stop"]);
    expect(c.state).toMatchObject({ active: false, running: false, motor: null });

    const d2 = fakeDeps();
    const c2 = new MotorTestController(d2);
    await c2.start(0, { port: "COM5", acks: ACKS });
    await c2.stop("requested");
    expect(d2.stop).toHaveBeenCalledTimes(1);
  });

  it("component unmount stops the motors (Esc step calls stop('unmount'))", async () => {
    const { d, c } = await running();
    await c.stop("unmount");
    expect(d.stop).toHaveBeenCalledTimes(1);
    expect(c.state.lastStop).toBe("unmount");
  });

  it("device://motor-test-stopped (WATCHDOG) ends the session and still sends device_motor_test_stop", async () => {
    const em = createEmitter();
    setEventSource(em.on);
    const { d, c } = await running();
    const off = bindMotorStopTriggers(c, { backendEvents: true });
    await flush();
    em.emit("device://motor-test-stopped", { reason: "WATCHDOG", error: null });
    await flush();
    expect(c.state.running).toBe(false);
    expect(c.state.lastStop).toBe("backend");
    expect(c.state.error).toMatchObject({ code: "MOTOR_TEST_NOT_ACTIVE" });
    expect(d.stop).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(d.keepalive).not.toHaveBeenCalled();
    off();
    resetEventSource();
  });

  it("maps every SCREAMING_SNAKE stop reason; the event's own error wins; REQUESTED while idle is not answered", async () => {
    const cases = [
      ["ARMED", "BOARD_ARMED"],
      ["DISCONNECTED", "DISCONNECTED"],
      ["ERROR", "UNKNOWN"],
      ["APP_CLOSING", null],
      ["REQUESTED", null],
    ] as const;
    for (const [reason, code] of cases) {
      const { d, c } = await running();
      await c.backendStopped(reason, null);
      expect(c.state.running, reason).toBe(false);
      expect(d.stop, reason).toHaveBeenCalledTimes(1);
      if (code) expect(c.state.error, reason).toMatchObject({ code });
      else expect(c.state.error, reason).toBeNull();
    }
    const { d, c } = await running();
    await c.backendStopped("ERROR", { code: "SERIAL", message: "write failed" });
    expect(c.state.error).toMatchObject({ code: "SERIAL_ERROR", message: "write failed" });
    expect(d.stop).toHaveBeenCalledTimes(1);
    // An idle controller still answers a real backend stop (releases the session) ...
    await c.backendStopped("WATCHDOG", null);
    expect(d.stop).toHaveBeenCalledTimes(2);
    // ... but not the echo of its own stop.
    await c.backendStopped("REQUESTED", null);
    expect(d.stop).toHaveBeenCalledTimes(2);
  });

  it("works end-to-end against the scenario mock (start, keepalives, stop on blur)", async () => {
    const mock = installMock({ latencyMs: 0 });
    const c = new MotorTestController(defaultMotorDeps);
    const target = new EventTarget();
    const off = bindMotorStopTriggers(c, { target });
    await c.start(0, { port: "COM5", acks: ACKS });
    await vi.advanceTimersByTimeAsync(400);
    target.dispatchEvent(new Event("blur"));
    await flush();
    expect(mock.device.motorLog).toEqual(["start 0 1100", "keepalive", "keepalive", "stop"]);
    expect(mock.device.motorsSpinning.size).toBe(0);
    off();
  });

  it("end-to-end: a backend stop event is followed by device_motor_test_stop", async () => {
    const mock = installMock({ latencyMs: 0 });
    const c = new MotorTestController(defaultMotorDeps);
    const off = bindMotorStopTriggers(c, { backendEvents: true });
    await flush();
    await c.start(2, { port: "COM5", acks: ACKS });
    mock.events.emit("device://motor-test-stopped", { reason: "ARMED", error: null });
    await flush();
    expect(mock.device.motorLog).toEqual(["start 2 1100", "stop"]);
    expect(c.state.error).toMatchObject({ code: "BOARD_ARMED" });
    off();
  });
});
