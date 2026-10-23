// SPDX-License-Identifier: GPL-3.0-or-later
// The live attitude stream and commands that need the board connection to themselves
// (Device #94: while the stream runs, only identify / sensors / get_attitude share the port;
// acc calibration, reboots, CLI, probe and Config's link commands get PORT_BUSY).
// The Calibration step stops the stream and waits for it before such a command, restarts it
// afterwards if the step is still visible, and copes with PORT_BUSY.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { attitudeStopping, SHARES_PORT_WITH_ATTITUDE, STOP_RETRY, stopAttitudeStream, withLinkGate } from "../src/api/attitudeLink";
import { api, EVENTS, resetBackend, setBackend, type Backend } from "../src/api/commands";
import { createEmitter, resetEventSource, setEventSource } from "../src/api/events";
import { resetDemoState } from "../src/api/fallback";
import { toUiError, uiError } from "../src/errors/codes";
import { i18n, setLocale } from "../src/i18n";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { installMock, uninstallMock } from "../src/mocks/install";
import type { Scenario, ScenarioMock } from "../src/mocks/scenarios";
import { nav, resetNav } from "../src/wizard/nav";
import { resetWizard, wizard } from "../src/wizard/state";
import { settle } from "./walk";

let mock: ScenarioMock;
let w: VueWrapper | null = null;
let visibility: "visible" | "hidden" = "visible";

beforeEach(() => {
  resetWizard();
  resetNav();
  resetDemoState();
  setLocale("en");
  visibility = "visible";
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
});
afterEach(async () => {
  STOP_RETRY.totalMs = 4000;
  w?.unmount();
  w = null;
  await settle();
  uninstallMock();
  resetBackend();
  resetEventSource();
  delete (document as unknown as Record<string, unknown>).visibilityState;
});

async function open(scenario: Scenario = "happy", setup?: (d: ScenarioMock["device"]) => void, stepMs = 0, latencyMs = 0): Promise<VueWrapper> {
  mock = installMock({ scenario, latencyMs, stepMs });
  setup?.(mock.device);
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await w.get('[data-test="next"]').trigger("click");
  await w.get('[data-test="connect"]').trigger("click");
  await waitFor(() => wizard.fc !== null);
  nav.currentId = "calibration";
  await settle();
  return w;
}
async function waitFor(pred: () => boolean, ms = 2000): Promise<void> {
  const end = Date.now() + ms;
  while (!pred() && Date.now() < end) {
    await new Promise((r) => setTimeout(r, 10));
    await settle();
  }
  expect(pred()).toBe(true);
}
const streaming = () => mock.device.attitudeTimer !== null;
const setVisibility = (v: "visible" | "hidden") => {
  visibility = v;
  document.dispatchEvent(new Event("visibilitychange"));
};

describe("Calibration step: the live view gives the port up for link commands", () => {
  it("calibrating stops the stream first (no PORT_BUSY), shows it paused, and restarts it afterwards", async () => {
    await open("happy", undefined, 30);
    await waitFor(streaming);
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => w!.find('[data-test="attitude-paused"]').exists());
    expect(streaming()).toBe(false);
    expect(w!.get('[data-test="attitude-paused"]').text()).toBe("The live view is paused while the board calibrates or restarts.");
    expect(w!.get('[data-test="calibrate"]').attributes("disabled")).toBeDefined();
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(mock.device.portBusyRefused).toEqual([]);
    expect(mock.device.accCalibrations).toBe(1);
    await waitFor(streaming);
    expect(mock.device.attitudeRates).toEqual([30, 30]);
    expect(w!.find('[data-test="attitude-paused"]').exists()).toBe(false);
  });

  it("a click while the stream is still starting waits for the start, then stops it (it never keeps running)", async () => {
    await open("happy", undefined, 0, 20);
    // As soon as the button can be pressed: the start call is still in flight.
    const btn = () => w!.find('[data-test="calibrate"]');
    await waitFor(() => btn().exists() && btn().attributes("disabled") === undefined, 3000);
    expect(streaming()).toBe(false);
    await btn().trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(mock.device.portBusyRefused).toEqual([]);
    await waitFor(streaming);
    await settle();
    expect(streaming()).toBe(true);
  });

  it("Reboot board: stream stopped before the reboot, restarted once the board is back", async () => {
    await open("unsaved-ram");
    await waitFor(streaming);
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    await waitFor(streaming); // a failed calibration brings the live view back too
    await w!.get('[data-error-code="PENDING_UNSAVED_CHANGES"]').findAll("button").find((x) => x.text() === "Reboot board")!.trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "idle");
    expect(mock.device.reboots).toBe(1);
    expect(mock.device.portBusyRefused).toEqual([]);
    await waitFor(streaming);
  });

  it("a reboot that lost the board does not restart the live view", async () => {
    await open("unsaved-ram", (d) => {
      d.rebootFailuresLeft = 1;
      d.rebootFailure = "lost";
    });
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    await waitFor(streaming);
    const before = mock.device.attitudeRates.length;
    await w!.get('[data-error-code="PENDING_UNSAVED_CHANGES"]').findAll("button").find((x) => x.text() === "Reboot board")!.trigger("click");
    await waitFor(() => w!.find('[data-error-code="REBOOT_BOARD_LOST"]').exists());
    await settle();
    expect(streaming()).toBe(false);
    expect(mock.device.attitudeRates.length).toBe(before);
  });

  it("PORT_BUSY once (the port not let go yet): the app frees the port and tries once more", async () => {
    await open();
    await waitFor(streaming);
    mock.device.portBusyOnce = 1;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(mock.device.portBusyRefused).toEqual(["device_acc_calibrate"]);
    expect(mock.device.accCalibrations).toBe(1);
    await waitFor(streaming);
  });

  it("PORT_BUSY again: a plain-language 'this app is using it' message with Try again; the live view comes back", async () => {
    await open();
    await waitFor(streaming);
    mock.device.portBusyOnce = 2;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    const panel = w!.get('[data-error-code="PORT_BUSY"]');
    expect(panel.text()).toContain("This app is already using the flight controller");
    await waitFor(streaming);
    await panel.findAll("button").find((x) => x.text() === "Try again")!.trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    await waitFor(streaming);
  });

  it("window hidden while calibrating: no restart until it is visible again", async () => {
    await open("happy", undefined, 30);
    await waitFor(streaming);
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "running" && !streaming());
    setVisibility("hidden");
    await waitFor(() => wizard.ui.accCalibration === "done");
    await settle();
    expect(streaming()).toBe(false);
    setVisibility("visible");
    await waitFor(streaming);
  });

  it("step left while calibrating: the stream is not restarted", async () => {
    await open("happy", undefined, 40);
    await waitFor(streaming);
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "running" && !streaming());
    nav.currentId = "beepers";
    await settle();
    await new Promise((r) => setTimeout(r, 200));
    await settle();
    expect(streaming()).toBe(false);
  });

  it("the stop answers PORT_BUSY, then OK: it is retried, and only then is the calibration sent", async () => {
    await open();
    await waitFor(streaming);
    mock.device.stopBusyLeft = 2;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done", 4000);
    expect(mock.device.attitudeStopCalls).toBe(3);
    expect(mock.device.portBusyRefused).toEqual([]);
    expect(mock.device.accCalibrations).toBe(1);
    await waitFor(streaming);
  });

  it("the stop stays PORT_BUSY: 'The board is busy, try again', no calibration sent, the live view carries on", async () => {
    STOP_RETRY.totalMs = 400;
    await open();
    await waitFor(streaming);
    mock.device.stopBusyLeft = Infinity;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed", 3000);
    const panel = w!.get('[data-error-code="LIVE_VIEW_BUSY"]');
    expect(panel.text()).toContain("The board is busy");
    expect(panel.text()).toContain("try again");
    expect(mock.device.accCalibrations).toBe(0);
    expect(mock.device.portBusyRefused).toEqual([]); // device_acc_calibrate was never sent
    expect(mock.device.attitudeStopCalls).toBeGreaterThan(1);
    expect(streaming()).toBe(true);
    await settle();
    expect(w!.find('[data-test="attitude-paused"]').exists()).toBe(false);
    // The board lets go: Try again calibrates.
    mock.device.stopBusyLeft = 0;
    await panel.findAll("button").find((x) => x.text() === "Try again")!.trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(mock.device.accCalibrations).toBe(1);
    await waitFor(streaming);
  });

  it("the stop stays PORT_BUSY before Reboot board: no reboot sent; Try again reboots", async () => {
    await open("unsaved-ram");
    await waitFor(streaming);
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    await waitFor(streaming);
    STOP_RETRY.totalMs = 400;
    mock.device.stopBusyLeft = Infinity;
    await w!.get('[data-error-code="PENDING_UNSAVED_CHANGES"]').findAll("button").find((x) => x.text() === "Reboot board")!.trigger("click");
    await waitFor(() => w!.find('[data-error-code="LIVE_VIEW_BUSY"]').exists(), 3000);
    expect(mock.device.reboots).toBe(0);
    expect(streaming()).toBe(true);
    mock.device.stopBusyLeft = 0;
    await w!.get('[data-error-code="LIVE_VIEW_BUSY"]').findAll("button").find((x) => x.text() === "Try again")!.trigger("click");
    await waitFor(() => mock.device.reboots === 1);
    await waitFor(() => wizard.ui.accCalibration === "idle");
    await waitFor(streaming);
  });

  it("the stop answers PORT_BUSY but device://attitude-stopped arrives: that counts as stopped", async () => {
    await open();
    await waitFor(streaming);
    mock.device.stopBusyLeft = Infinity;
    mock.device.stopBusyStopsAnyway = true;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(mock.device.attitudeStopCalls).toBe(1);
    expect(mock.device.accCalibrations).toBe(1);
    expect(mock.device.portBusyRefused).toEqual([]);
  });

  it("Hebrew: the board-busy error", async () => {
    setLocale("he");
    STOP_RETRY.totalMs = 300;
    await open();
    await waitFor(streaming);
    mock.device.stopBusyLeft = Infinity;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => w!.find('[data-error-code="LIVE_VIEW_BUSY"]').exists(), 3000);
    const text = w!.get('[data-error-code="LIVE_VIEW_BUSY"]').text();
    expect(text).toContain("הלוח עסוק");
    expect(text).toContain("נסו שוב");
    expect(mock.device.accCalibrations).toBe(0);
  });

  it("Hebrew: the paused note", async () => {
    setLocale("he");
    await open("happy", undefined, 40);
    await waitFor(streaming);
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => w!.find('[data-test="attitude-paused"]').exists());
    expect(w!.get('[data-test="attitude-paused"]').text()).toBe("התצוגה החיה מושהית בזמן שהלוח מתכייל או מופעל מחדש.");
    expect(he.calibration.orientation.paused).toBeTruthy();
    expect(en.calibration.orientation.paused).toBeTruthy();
  });
});

describe("attitudeLink: stop and wait, link gate", () => {
  function fakeBackend(stopMs: number | "never") {
    const emitter = createEmitter();
    const log: string[] = [];
    const b = (async (cmd: string) => {
      log.push(`>${cmd}`);
      if (cmd === "device_attitude_stream_stop") {
        if (stopMs === "never") return new Promise(() => undefined);
        await new Promise((r) => setTimeout(r, stopMs));
      }
      log.push(`<${cmd}`);
      return null;
    }) as unknown as Backend;
    setBackend(withLinkGate(b));
    setEventSource(emitter.on);
    return { emitter, log };
  }

  it("a link command waits for the stop in flight; identify / sensors do not", async () => {
    const { log } = fakeBackend(30);
    const stop = stopAttitudeStream();
    const cal = api.device.accCalibrate();
    const id = api.device.identify();
    await id;
    expect(log).toContain("<device_identify");
    expect(log).not.toContain(">device_acc_calibrate");
    await stop;
    await cal;
    expect(log.indexOf("<device_attitude_stream_stop")).toBeLessThan(log.indexOf(">device_acc_calibrate"));
    expect(SHARES_PORT_WITH_ATTITUDE).toEqual(expect.arrayContaining(["device_identify", "device_sensors", "device_get_attitude"]));
  });

  it("device://attitude-stopped ends the wait even if the stop call has not answered", async () => {
    const { emitter } = fakeBackend("never");
    let done = false;
    const stop = stopAttitudeStream().then(() => (done = true));
    await new Promise((r) => setTimeout(r, 20));
    expect(done).toBe(false);
    emitter.emit(EVENTS.attitudeStopped, { reason: "REQUESTED", error: null });
    await stop;
    expect(done).toBe(true);
    await attitudeStopping(); // nothing in flight any more
  });

  /** A backend whose stop answers per call: "busy" (PORT_BUSY), "gone" (DISCONNECTED) or "ok". */
  function stopScript(next: (n: number) => "busy" | "gone" | "ok") {
    const emitter = createEmitter();
    const log: string[] = [];
    let n = 0;
    const b = (async (cmd: string) => {
      log.push(`>${cmd}`);
      if (cmd === "device_attitude_stream_stop") {
        const r = next(n++);
        await new Promise((res) => setTimeout(res, 5));
        if (r === "busy") throw { ...uiError("PORT_BUSY", "shared call"), details: { port: "COM5", heldBy: "APP" } };
        if (r === "gone") throw uiError("DISCONNECTED", "not connected");
      }
      log.push(`<${cmd}`);
      return null;
    }) as unknown as Backend;
    setBackend(withLinkGate(b));
    setEventSource(emitter.on);
    return { emitter, log, calls: () => n };
  }

  it("PORT_BUSY, PORT_BUSY, OK: retried with backoff; a link command waits until it really stopped", async () => {
    const { log, calls } = stopScript((n) => (n < 2 ? "busy" : "ok"));
    const stop = stopAttitudeStream();
    const cal = api.device.accCalibrate();
    await stop;
    await cal;
    expect(calls()).toBe(3);
    expect(log.lastIndexOf("<device_attitude_stream_stop")).toBeLessThan(log.indexOf(">device_acc_calibrate"));
  });

  it("always PORT_BUSY: rejects with LIVE_VIEW_BUSY after the bounded wait (and the gate does not hang)", async () => {
    STOP_RETRY.totalMs = 300;
    const { calls } = stopScript(() => "busy");
    const t0 = Date.now();
    const err = await stopAttitudeStream().then(() => null, (e) => e);
    const took = Date.now() - t0;
    expect(toUiError(err).code).toBe("LIVE_VIEW_BUSY");
    expect(took).toBeGreaterThanOrEqual(250);
    expect(took).toBeLessThan(1500);
    expect(calls()).toBeGreaterThan(2);
    await attitudeStopping(); // resolves, never rejects
  });

  it("the default bound is between 3 and 5 seconds", () => {
    expect(STOP_RETRY.totalMs).toBeGreaterThanOrEqual(3000);
    expect(STOP_RETRY.totalMs).toBeLessThanOrEqual(5000);
  });

  it("PORT_BUSY, then device://attitude-stopped during the backoff: done without another try", async () => {
    const { emitter, calls } = stopScript(() => "busy");
    const stop = stopAttitudeStream();
    await new Promise((r) => setTimeout(r, 15));
    emitter.emit(EVENTS.attitudeStopped, { reason: "REQUESTED", error: null });
    await stop;
    expect(calls()).toBe(1);
  });

  it("any other stop error (not connected, no stream) means there is nothing to stop", async () => {
    const { calls } = stopScript(() => "gone");
    await stopAttitudeStream();
    expect(calls()).toBe(1);
  });
});
