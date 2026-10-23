// SPDX-License-Identifier: GPL-3.0-or-later
// Calibration and orientation step: sensors (extra keys tolerated), toggles and
// warnings, accelerometer calibration success / failure / retry, the attitude stream
// (PORT_BUSY), alignment presets with the client-side preview, and null = keep.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { demoState, installFallback, resetDemoState } from "../src/api/fallback";
import { parseSensors, pendingApi } from "../src/api/pending";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import { createScenarioMock, type Scenario, type ScenarioMock } from "../src/mocks/scenarios";
import { calibrationWarnings, guards } from "../src/wizard/guards";
import { simulatedFields, toPlanModel, unplannedChoices } from "../src/wizard/modelExt";
import { nav, resetNav } from "../src/wizard/nav";
import { addModeSwitch, createWizardState, resetWizard, wizard } from "../src/wizard/state";
import { settle } from "./walk";

let mock: ScenarioMock;
let w: VueWrapper | null = null;

beforeEach(() => {
  resetWizard();
  resetNav();
  resetDemoState();
  setLocale("en");
});
afterEach(async () => {
  w?.unmount();
  w = null;
  await settle();
  uninstallMock();
});

async function open(scenario: Scenario = "happy"): Promise<VueWrapper> {
  mock = installMock({ scenario, latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await w.get('[data-test="next"]').trigger("click");
  await w.get('[data-test="connect"]').trigger("click");
  await settle();
  nav.currentId = "calibration";
  await settle();
  return w;
}
const waitFor = async (pred: () => boolean, ms = 1500) => {
  const end = Date.now() + ms;
  while (!pred() && Date.now() < end) {
    await new Promise((r) => setTimeout(r, 20));
    await settle();
  }
  expect(pred()).toBe(true);
};

describe("sensors", () => {
  it("parseSensors ignores unknown keys and treats missing sensors as absent", () => {
    const s = parseSensors({ gyro: { present: true }, acc: { present: true, enabled: true }, rangefinder: { present: true }, future: 1 });
    expect(s).toEqual({ gyro: { present: true }, acc: { present: true, enabled: true }, baro: { present: false, enabled: false }, mag: { present: false, enabled: false } });
    expect(parseSensors(null).gyro.present).toBe(false);
  });

  it("shows the detected sensors; gyro always on; baro only when present", async () => {
    await open();
    expect(wizard.ui.sensors?.acc.present).toBe(true);
    expect(w!.get('[data-sensor="gyro"]').text()).toContain("Always on");
    expect(w!.find('[data-test="acc-toggle"]').exists()).toBe(true);
    expect(w!.find('[data-test="baro-toggle"]').exists()).toBe(true);
    // Untouched = keep.
    expect(wizard.ext.sensors).toBeNull();
  });

  it("no-sensors scenario: no accelerometer toggle, no baro row, no calibration", async () => {
    await open("no-sensors");
    expect(w!.find('[data-test="acc-toggle"]').exists()).toBe(false);
    expect(w!.find('[data-sensor="baro"]').exists()).toBe(false);
    expect(w!.find('[data-section="acc-calibration"]').exists()).toBe(false);
    expect(w!.get('[data-sensor="acc"]').text()).toContain("Not on this board");
  });

  it("first toggle sets both sensors explicitly; ANGLE with acc off warns (not blocking)", async () => {
    await open();
    addModeSwitch(wizard, "angle");
    await w!.get('[data-test="acc-toggle"]').setValue(false);
    expect(wizard.ext.sensors).toEqual({ acc: false, baro: false });
    expect(w!.find('[data-warning="calibration.warnAngleNoAcc"]').exists()).toBe(true);
    await w!.get('[data-test="baro-toggle"]').setValue(true);
    expect(wizard.ext.sensors).toEqual({ acc: false, baro: true });
    expect(guards.calibration!.validate(wizard)).toEqual([]);
  });

  it("calibrationWarnings: ANGLE / GPS without acc", () => {
    const s = createWizardState();
    expect(calibrationWarnings(s)).toEqual([]);
    s.ext.sensors = { acc: false, baro: false };
    expect(calibrationWarnings(s)).toEqual([]);
    addModeSwitch(s, "angle");
    s.model.gps.enabled = true;
    expect(calibrationWarnings(s).map((i) => i.key)).toEqual(["calibration.warnAngleNoAcc", "calibration.warnGpsNoAcc"]);
  });
});

describe("accelerometer calibration", () => {
  it("success", async () => {
    await open();
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(w!.find('[data-test="calibrated"]').exists()).toBe(true);
    expect(mock.device.accCalibrations).toBe(1);
  });

  it("failure shows guidance and a retry that then succeeds", async () => {
    await open("calib-fail");
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    expect(w!.find('[data-test="calibration-failed"]').exists()).toBe(true);
    expect(w!.find("[data-error-code]").exists()).toBe(true);
    expect(w!.get('[data-test="calibrate"]').text()).toBe("Try again");
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(mock.device.accCalibrations).toBe(1);
  });

  it("BOARD_ARMED and ACC_NOT_AVAILABLE get their own guidance", async () => {
    await open();
    mock.device.armed = true;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    expect(w!.find('[data-error-code="BOARD_ARMED"]').exists()).toBe(true);
    expect(w!.text()).toContain("motor test or calibration");
    mock.device.armed = false;
    mock.device.sensors.acc.enabled = false;
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => !!w!.find('[data-error-code="ACC_NOT_AVAILABLE"]').exists());
    expect(w!.text()).toContain("The accelerometer is not available");
  });

  it("refused with MIXED_BACKEND when the board is real but calibration would be simulated", async () => {
    const real = createScenarioMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    const sim = createScenarioMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    const notImpl = { code: "NOT_IMPLEMENTED", message: "x" };
    const realBackend = (async (cmd: string, args: never) => {
      if (cmd.startsWith("device_acc") || cmd === "device_sensors" || cmd.startsWith("device_attitude")) throw notImpl;
      return real.backend(cmd as never, args);
    }) as unknown as typeof real.backend;
    installFallback({ backend: sim.backend, events: sim.events.on }, { backend: realBackend, events: real.events.on });
    await real.backend("device_connect" as never, { path: "COM5" } as never);
    demoState.sources.device_connect = "real";
    await expect(pendingApi.accCalibrate()).rejects.toMatchObject({ code: "MIXED_BACKEND" });
    expect(sim.device.accCalibrations).toBe(0);
    // Sensors read from the demo on a real board: the answers are never written.
    await pendingApi.sensors();
    expect(simulatedFields()).toContain("sensors");
    const s = createWizardState();
    s.ext.sensors = { acc: false, baro: false };
    expect("sensors" in toPlanModel(s.model, s.ext)).toBe(false);
  });
});

describe("orientation", () => {
  it("the attitude stream drives the view; nose down shows nose down", async () => {
    await open();
    mock.device.tilt = { roll: 0, pitch: -30, yaw: 0 };
    await waitFor(() => w!.get('[data-test="attitude-view"]').attributes("data-direction") === "noseDown");
    expect(mock.device.attitudeTimer).not.toBeNull();
    expect(mock.device.rcTimer).toBeNull();
  });

  it("mounted 90° yawed: nose down looks like a roll until the yaw 90 preset is chosen", async () => {
    await open("mounted-yaw90");
    mock.device.tilt = { roll: 0, pitch: -30, yaw: 0 };
    const dir = () => w!.get('[data-test="attitude-view"]').attributes("data-direction");
    await waitFor(() => dir() !== "" && dir() !== "level");
    expect(dir()).not.toBe("noseDown");
    await w!.get('[data-orientation="no"]').setValue(true);
    expect(w!.find('[data-test="alignment-presets"]').exists()).toBe(true);
    await w!.get('[data-preset="yaw90"]').trigger("click");
    expect(wizard.ext.boardAlignment).toEqual({ roll: 0, pitch: 0, yaw: 90 });
    await waitFor(() => dir() === "noseDown");
    mock.device.tilt = { roll: 30, pitch: 0, yaw: 0 };
    await waitFor(() => dir() === "rightDown");
    expect(toPlanModel(wizard.model, wizard.ext).boardAlignment).toEqual({ roll: 0, pitch: 0, yaw: 90 });
    // Client-side only: nothing was sent to the board yet.
    expect(mock.device.mount).toEqual({ roll: 0, pitch: 0, yaw: 90 });
    await w!.get('[data-test="alignment-keep"]').trigger("click");
    expect(wizard.ext.boardAlignment).toBeNull();
  });

  it("PORT_BUSY from the attitude stream is shown with a retry", async () => {
    mock = installMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    const inner = mock.backend;
    let busy = true;
    const { setBackend } = await import("../src/api/commands");
    setBackend((async (cmd: string, args: never) => {
      if (cmd === "device_attitude_stream_start" && busy) throw { code: "PORT_BUSY", message: "motor test running", details: { port: "COM5", heldBy: "APP" } };
      return inner(cmd as never, args);
    }) as unknown as typeof inner);
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    await w.get('[data-test="next"]').trigger("click");
    await w.get('[data-test="connect"]').trigger("click");
    await settle();
    nav.currentId = "calibration";
    await settle();
    expect(w.find('[data-test="attitude-busy"]').exists()).toBe(true);
    busy = false;
    await w.get('[data-test="attitude-retry"]').trigger("click");
    await settle();
    expect(w.find('[data-test="attitude-view"]').exists()).toBe(true);
  });

  it("the mock refuses the RC stream / motor test while the attitude stream runs", async () => {
    const m = createScenarioMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    await m.backend("device_attitude_stream_start" as never, { rateHz: 10 } as never);
    await expect(m.backend("device_rc_stream_start" as never, { rateHz: 10 } as never)).rejects.toMatchObject({ code: "PORT_BUSY" });
    await m.backend("device_attitude_stream_stop" as never, {} as never);
    await m.backend("device_rc_stream_start" as never, { rateHz: 10 } as never);
    await expect(m.backend("device_attitude_stream_start" as never, { rateHz: 10 } as never)).rejects.toMatchObject({ code: "PORT_BUSY" });
    await m.backend("device_rc_stream_stop" as never, {} as never);
  });

  it("leaving the step stops the attitude stream", async () => {
    await open();
    await waitFor(() => mock.device.attitudeTimer !== null);
    nav.currentId = "beepers";
    await settle();
    expect(mock.device.attitudeTimer).toBeNull();
  });
});

describe("plan adapter", () => {
  it("untouched answers stay null and are not sent; explicit ones are", () => {
    const s = createWizardState();
    expect(Object.keys(toPlanModel(s.model, s.ext)).sort()).toEqual(Object.keys(s.model).sort());
    s.ext.boardAlignment = { roll: 180, pitch: 0, yaw: 0 };
    s.ext.beepers = { disabled: ["ON_USB"] };
    const p = toPlanModel(s.model, s.ext);
    expect(p.boardAlignment).toEqual({ roll: 180, pitch: 0, yaw: 0 });
    expect(p.beepers).toEqual({ disabled: ["ON_USB"] });
    expect(p.airmode).toBeUndefined();
  });

  it("unplannedChoices lists explicit answers the plan does not write", () => {
    const s = createWizardState();
    s.ext.airmode = "always";
    s.ext.beepers = { disabled: [] };
    expect(unplannedChoices(s.ext, { commands: ["feature AIRMODE", "save"], warnings: [], dialect: "4.5", cleared: [] } as never)).toEqual(["beepers"]);
    expect(unplannedChoices(s.ext, null)).toEqual([]);
  });
});

describe("RTL", () => {
  it("Hebrew: the page is RTL, the quad pictures and CLI names stay LTR", async () => {
    setLocale("he");
    await open();
    expect(document.documentElement.dir).toBe("rtl");
    expect(w!.get('[data-test="attitude-view"]').attributes("dir")).toBe("ltr");
    expect(w!.text()).toContain("חיישנים ויישור");
    expect(w!.text()).toContain("מד תאוצה");
    nav.currentId = "beepers";
    await settle();
    await w!.get('[data-beepers="choose"]').setValue(true);
    await settle();
    expect(w!.text()).toContain("חזרה להמלצה");
    expect(w!.get("small.mono").attributes("dir")).toBe("ltr");
    nav.currentId = "switches";
    await settle();
    expect(w!.text()).toContain("דילוג, לא צריך את זה");
  });
});
