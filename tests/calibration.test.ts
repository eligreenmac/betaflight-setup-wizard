// SPDX-License-Identifier: GPL-3.0-or-later
// Calibration and orientation step: sensors (extra keys tolerated), toggles and
// warnings, accelerometer calibration success / failure / retry, the attitude stream
// (PORT_BUSY), alignment presets with the client-side preview, and null = keep.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { demoState, installFallback, resetDemoState } from "../src/api/fallback";
import { attitudeToEuler, parseSensors, pendingApi } from "../src/api/pending";
import { categoryOf, toUiError } from "../src/errors/codes";
import { guidanceFor } from "../src/errors/guidance";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import { createScenarioMock, type Scenario, type ScenarioMock } from "../src/mocks/scenarios";
import { calibrationBlock, calibrationWarnings, guards, sensorIssues } from "../src/wizard/guards";
import { generateCli } from "../src/mocks/scenarios";
import { isWriteCommand } from "../src/api/fallback";
import { ALL_STEPS } from "../src/wizard/engine";
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

async function open(scenario: Scenario = "happy", setup?: (d: ScenarioMock["device"]) => void, stepMs = 0): Promise<VueWrapper> {
  mock = installMock({ scenario, latencyMs: 0, stepMs });
  setup?.(mock.device);
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
  it("parseSensors reads SensorStatus (#63), ignores unknown keys, missing sensors = not found", () => {
    const s = parseSensors({
      gyro: { present: true, enabled: true, configuredHardware: null, detectedHardware: "BMI270" },
      acc: { present: true, enabled: true, configuredHardware: "AUTO", detectedHardware: "BMI270" },
      baro: { present: false, enabled: false, configuredHardware: "NONE", detectedHardware: null },
      accCalibrated: false,
      alignment: { boardRollDeg: 0, boardPitchDeg: 0, boardYawDeg: 90, gyroAlignment: "CW270" },
      rangefinder: { present: true },
      future: 1,
    });
    expect(s).toEqual({
      gyro: { present: true },
      acc: { present: true, enabled: true },
      baro: { present: false, enabled: false }, // turned off in the config
      mag: { present: false, enabled: true }, // missing = not found (no toggle)
      accCalibrated: false,
      alignment: { roll: 0, pitch: 0, yaw: 90 },
    });
    const empty = parseSensors(null);
    expect(empty.gyro.present).toBe(false);
    expect([empty.accCalibrated, empty.alignment]).toEqual([null, null]);
    expect(parseSensors({ alignment: { boardRollDeg: 0 } }).alignment).toBeNull();
  });

  it("a sensor turned off in the config can be turned on; one not found cannot", async () => {
    await open();
    expect(wizard.ui.sensors?.baro).toEqual({ present: false, enabled: false });
    expect(w!.find('[data-test="baro-toggle"]').exists()).toBe(true);
    expect((w!.get('[data-test="baro-toggle"]').element as HTMLInputElement).checked).toBe(false);
    expect(w!.find('[data-sensor="baro"]').text()).toContain("Turned off in the board's settings");
    expect(w!.find('[data-test="needs-calibration"]').exists()).toBe(true);
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

  it("only the touched sensor is set; acc off + ANGLE is a blocking error with fixes", async () => {
    await open();
    addModeSwitch(wizard, "angle");
    await w!.get('[data-test="acc-toggle"]').setValue(false);
    expect(wizard.ext.sensors).toEqual({ acc: false, baro: null, mag: null });
    expect(toPlanModel(wizard.model, wizard.ext).sensors).toEqual({ acc: false });
    expect(guards.calibration!.validate(wizard)).toEqual([{ key: "validation.accNeededByModes", params: { modes: "ANGLE" } }]);
    expect(w!.find('[data-error="validation.accNeededByModes"]').exists()).toBe(true);
    // Fix 1: turn the acc back on (the FC has it on: back to keep).
    await w!.get('[data-test="fix-acc-on"]').trigger("click");
    expect(wizard.ext.sensors?.acc).toBeNull();
    expect(guards.calibration!.validate(wizard)).toEqual([]);
    // Fix 2: remove the switches that need it.
    await w!.get('[data-test="acc-toggle"]').setValue(false);
    await w!.get('[data-test="fix-remove-modes"]').trigger("click");
    expect(wizard.model.switches.map((x) => x.mode)).toEqual(["ARM"]);
    expect(guards.calibration!.validate(wizard)).toEqual([]);
    await w!.get('[data-test="baro-toggle"]').setValue(true);
    expect(wizard.ext.sensors).toEqual({ acc: false, baro: true, mag: null });
  });

  it("acc:false blocks with every acc mode (ANGLE, HORIZON, ALTHOLD, POSHOLD, GPS RESCUE), not with others", () => {
    for (const k of ["angle", "horizon", "altHold", "posHold", "gpsRescue"] as const) {
      const s = createWizardState();
      s.ext.sensors = { acc: false, baro: null, mag: null };
      addModeSwitch(s, k);
      expect(sensorIssues(s).map((i) => i.key)).toEqual(["validation.accNeededByModes"]);
      expect(guards.calibration!.validate(s).map((i) => i.key)).toContain("validation.accNeededByModes");
    }
    const s = createWizardState();
    s.ext.sensors = { acc: false, baro: null, mag: null };
    addModeSwitch(s, "beeper");
    addModeSwitch(s, "turtle");
    expect(sensorIssues(s)).toEqual([]);
    // acc kept (null) on a board without one: only a warning.
    const t2 = createWizardState();
    t2.ui.sensors = { gyro: { present: true }, acc: { present: false, enabled: true }, baro: { present: false, enabled: true }, mag: { present: false, enabled: true }, accCalibrated: null, alignment: null };
    addModeSwitch(t2, "angle");
    expect(sensorIssues(t2)).toEqual([]);
    expect(calibrationWarnings(t2).map((i) => i.key)).toEqual(["calibration.warnAngleNoAcc"]);
  });

  it("calibrationWarnings: GPS without acc; baro:false + ALTHOLD is a warning", () => {
    const s = createWizardState();
    expect(calibrationWarnings(s)).toEqual([]);
    s.ext.sensors = { acc: false, baro: false, mag: null };
    expect(calibrationWarnings(s)).toEqual([]);
    s.model.gps.enabled = true;
    expect(calibrationWarnings(s).map((i) => i.key)).toEqual(["calibration.warnGpsNoAcc"]);
    s.ext.sensors = { acc: null, baro: false, mag: null };
    addModeSwitch(s, "altHold");
    expect(calibrationWarnings(s).map((i) => i.key)).toEqual(["calibration.warnAltholdNoBaro"]);
    expect(guards.calibration!.validate(s)).toEqual([]);
  });

  it("magnetometer toggle only when one is present", async () => {
    await open();
    expect(w!.find('[data-test="mag-toggle"]').exists()).toBe(false);
    w!.unmount();
    w = null;
    uninstallMock();
    resetWizard();
    resetNav();
    const m = await open("happy", (d) => (d.sensors.mag = { present: true, enabled: true }));
    void m;
    await w!.get('[data-test="mag-toggle"]').setValue(false);
    expect(wizard.ext.sensors).toEqual({ acc: null, baro: null, mag: false });
    expect(generateCli(toPlanModel(wizard.model, wizard.ext), []).commands).toContain("set mag_hardware = NONE");
  });

  it("calibration is saved right away (one line) and waits after a failed apply", async () => {
    await open();
    expect(w!.get('[data-test="saves-now"]').text()).toContain("saved on the board right away");
    wizard.ui.applyStatus = "failed";
    await settle();
    expect(w!.find('[data-test="unsaved-changes"]').exists()).toBe(true);
    expect(w!.get('[data-test="calibrate"]').attributes("disabled")).toBeDefined();
  });

  it("no step before calibration writes settings to the board", () => {
    // Acc calibration saves the FC's RAM to EEPROM. Only Apply writes settings (config_apply),
    // and Apply comes after Calibration.
    const ids = ALL_STEPS.map((x) => x.id);
    expect(ids.indexOf("apply")).toBeGreaterThan(ids.indexOf("calibration"));
    expect(isWriteCommand("config_apply")).toBe(true);
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
    expect(w!.find('[data-error-code="BOARD_MOVING"]').exists()).toBe(true);
    expect(w!.text()).toContain("The quad moved while calibrating");
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
    expect(w!.text()).toContain("The accelerometer is turned off"); // details.enabled = false
    mock.device.sensors.acc = { present: false, enabled: true };
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => w!.text().includes("No accelerometer found"));
  });

  it("BOARD_MOVING (during) says calibrate again; ACC_CALIBRATION_FAILED has its own text", () => {
    expect(guidanceFor({ code: "BOARD_MOVING", message: "", details: { maxRateDps: 20, limitDps: 8, during: true } } as never).titleKey).toBe("errors.BOARD_MOVING_DURING.title");
    expect(guidanceFor({ code: "BOARD_MOVING", message: "", details: { maxRateDps: 20, limitDps: 8, during: false } } as never).titleKey).toBe("errors.BOARD_MOVING.title");
    expect(guidanceFor("ACC_CALIBRATION_FAILED").titleKey).toBe("errors.ACC_CALIBRATION_FAILED.title");
    expect(toUiError({ code: "ACC_CALIBRATION_FAILED", message: "x" }).code).toBe("ACC_CALIBRATION_FAILED");
  });

  it("PENDING_UNSAVED_CHANGES {reason} (TODO(shared) string code): Reboot board + Try again; REBOOT_NOT_CONFIRMED: Go to Connect", () => {
    const e = toUiError({ code: "PENDING_UNSAVED_CHANGES", message: "x", details: { reason: "EXIT_NOREBOOT" } });
    expect(e).toMatchObject({ code: "PENDING_UNSAVED_CHANGES", category: "Safety", details: { reason: "EXIT_NOREBOOT" } });
    const g = guidanceFor(e);
    expect(g.titleKey).toBe("errors.PENDING_UNSAVED_CHANGES.title");
    expect(g.actions.map((x) => [x.id, x.labelKey])).toEqual([["rebootBoard", "actions.rebootBoard"], ["retry", "actions.tryAgain"]]);
    const r = guidanceFor("REBOOT_NOT_CONFIRMED");
    expect(categoryOf("REBOOT_NOT_CONFIRMED")).toBe("Connection");
    expect(r.actions.map((x) => x.id)).toEqual(["goConnect", "retry"]);
  });

  it("unsaved changes: refused first (even armed), a plain reconnect does not clear it, Reboot board does", async () => {
    await open("unsaved-ram", (d) => (d.armed = true), 40);
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    const panel = () => w!.get('[data-error-code="PENDING_UNSAVED_CHANGES"]');
    expect(panel().text()).toContain("The board has changes that were never saved, and calibrating now would save them too.");
    expect(panel().text()).toContain("it will restart and nothing unsaved will be kept");
    expect(w!.find('[data-test="calibration-failed"]').exists()).toBe(false); // not the "keep it still" hint
    // A reconnect alone keeps the flag (Device: only a confirmed reboot clears it).
    await mock.backend("device_disconnect", {});
    await mock.backend("device_connect", { path: "COM5" });
    expect(mock.device.unsavedRam).toBe(true);
    mock.device.armed = false;
    await waitFor(() => mock.device.attitudeTimer !== null);
    const reboot = panel().findAll("button").find((x) => x.text() === "Reboot board")!;
    await reboot.trigger("click");
    await waitFor(() => w!.find('[data-test="rebooting"]').exists());
    expect(w!.get('[data-test="rebooting"]').text()).toBe("Restarting the board…");
    expect(mock.device.attitudeTimer).toBeNull(); // live view paused during the reboot
    await waitFor(() => wizard.ui.accCalibration === "idle");
    expect(mock.device.reboots).toBe(1);
    expect(mock.device.unsavedRam).toBe(false);
    await waitFor(() => mock.device.attitudeTimer !== null); // and resumed
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    expect(mock.device.accCalibrations).toBe(1);
  });

  it("reboot not confirmed: power-cycle guidance, Try again reboots again, Go to Connect", async () => {
    await open("unsaved-ram-reboot-fail");
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    await w!.get('[data-error-code="PENDING_UNSAVED_CHANGES"]').findAll("button").find((x) => x.text() === "Reboot board")!.trigger("click");
    await waitFor(() => w!.find('[data-error-code="REBOOT_NOT_CONFIRMED"]').exists());
    const panel = () => w!.get('[data-error-code="REBOOT_NOT_CONFIRMED"]');
    expect(panel().text()).toContain("We couldn't confirm the board restarted. Unplug the USB cable and the battery, wait a few seconds, plug back in, and connect again.");
    expect(mock.device.unsavedRam).toBe(true);
    await panel().findAll("button").find((x) => x.text() === "Try again")!.trigger("click");
    await waitFor(() => !w!.find('[data-test="rebooting"]').exists() && w!.find('[data-error-code="REBOOT_NOT_CONFIRMED"]').exists());
    expect(mock.device.accCalibrations).toBe(0);
    await panel().findAll("button").find((x) => x.text() === "Go to Connect")!.trigger("click");
    await waitFor(() => nav.currentId === "connect");
    expect(wizard.fc).toBeNull();
    expect(wizard.connectedPath).toBeNull();
  });

  it("Connect step: PENDING_UNSAVED_CHANGES from device_connect = power-cycle guidance, then connect again", async () => {
    mock = installMock({ scenario: "unsaved-ram", latencyMs: 0, stepMs: 0 });
    mock.device.connectUnsavedFails = true;
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    await w.get('[data-test="next"]').trigger("click");
    await w.get('[data-test="connect"]').trigger("click");
    await waitFor(() => w!.find('[data-error-code="REBOOT_NOT_CONFIRMED"]').exists());
    const panel = w.get('[data-error-code="REBOOT_NOT_CONFIRMED"]');
    expect(panel.text()).toContain("We couldn't confirm the board restarted. Unplug the USB cable and the battery, wait a few seconds, plug back in, and connect again.");
    const labels = panel.findAll("button").map((x) => x.text());
    expect(labels).toEqual(["Try again"]); // no "Go to Connect" on the Connect step
    expect(wizard.fc).toBeNull();
    mock.device.connectUnsavedFails = false; // unplugged and plugged back in
    await panel.findAll("button")[0]!.trigger("click");
    await waitFor(() => wizard.fc !== null);
  });

  it("unsaved changes and reboot failure guidance in Hebrew", async () => {
    await open("unsaved-ram-reboot-fail");
    setLocale("he");
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "failed");
    const p = w!.get('[data-error-code="PENDING_UNSAVED_CHANGES"]');
    expect(p.text()).toContain("יש בלוח שינויים שמעולם לא נשמרו, וכיול עכשיו ישמור גם אותם.");
    await p.findAll("button").find((x) => x.text() === "הפעלה מחדש של הלוח")!.trigger("click");
    await waitFor(() => w!.find('[data-error-code="REBOOT_NOT_CONFIRMED"]').exists());
    expect(w!.text()).toContain("לא הצלחנו לוודא שהלוח הופעל מחדש. נתקו את כבל ה-USB ואת הסוללה");
    expect(w!.text()).toContain("מעבר לשלב החיבור");
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
    s.ext.sensors = { acc: false, baro: false, mag: null };
    expect("sensors" in toPlanModel(s.model, s.ext)).toBe(false);
  });
});

describe("config rule: calibrate only before config_plan or after config_apply", () => {
  it("calibrationBlock: plan pending / applying / after apply returned", () => {
    const s = createWizardState();
    expect(calibrationBlock(s)).toBeNull();
    s.ui.script = { commands: ["save"], cleared: [], warnings: [] } as never;
    expect(calibrationBlock(s)).toBe("planPending");
    s.ui.applyStatus = "applying";
    expect(calibrationBlock(s)).toBe("applying");
    for (const st of ["verifying", "verified", "mismatch", "failed"] as const) {
      s.ui.applyStatus = st;
      expect(calibrationBlock(s)).toBeNull();
    }
  });

  it("while config_apply runs, Calibrate is disabled with an explanation", async () => {
    await open();
    wizard.ui.script = { commands: ["save"], cleared: [], warnings: [] } as never;
    wizard.ui.applyStatus = "applying";
    await settle();
    expect(w!.get('[data-test="calibrate"]').attributes("disabled")).toBeDefined();
    expect(w!.get('[data-test="calibration-blocked-applying"]').text()).toContain("Settings are being written to the board");
    wizard.ui.applyStatus = "verified"; // config_apply returned
    await settle();
    expect(w!.get('[data-test="calibrate"]').attributes("disabled")).toBeUndefined();
    expect(wizard.ui.script).not.toBeNull(); // an applied plan is kept
  });

  it("a plan made while on the step blocks Calibrate (safety net)", async () => {
    await open();
    wizard.ui.script = { commands: ["save"], cleared: [], warnings: [] } as never;
    await settle();
    expect(w!.get('[data-test="calibrate"]').attributes("disabled")).toBeDefined();
    expect(w!.find('[data-test="calibration-blocked-plan"]').exists()).toBe(true);
  });

  it("back from Review to Calibration drops the unapplied plan; Review plans again", async () => {
    await open();
    nav.currentId = "review";
    await waitFor(() => wizard.ui.script !== null);
    nav.currentId = "calibration";
    await settle();
    expect(wizard.ui.script).toBeNull();
    expect(w!.get('[data-test="plan-dropped"]').text()).toContain("so that plan was dropped");
    expect(w!.get('[data-test="calibrate"]').attributes("disabled")).toBeUndefined();
    await w!.get('[data-test="calibrate"]').trigger("click");
    await waitFor(() => wizard.ui.accCalibration === "done");
    nav.currentId = "review";
    await waitFor(() => wizard.ui.script !== null);
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

  it("attitude arrives with MSP signs (pitch + = nose down) and is converted", () => {
    expect(attitudeToEuler({ roll: 10, pitch: 30, yaw: 5 })).toEqual({ roll: 10, pitch: -30, yaw: 5 });
  });

  it("the FC's current alignment is undone for the preview", async () => {
    await open("mounted-yaw90");
    // The board already has yaw 90 stored: the reported attitude is right, and keeping it previews correctly.
    mock.device.alignment = { roll: 0, pitch: 0, yaw: 90 };
    nav.currentId = "beepers";
    await settle();
    nav.currentId = "calibration";
    await settle();
    expect(wizard.ui.sensors?.alignment).toEqual({ roll: 0, pitch: 0, yaw: 90 });
    mock.device.tilt = { roll: 0, pitch: -30, yaw: 0 };
    const dir = () => w!.get('[data-test="attitude-view"]').attributes("data-direction");
    await waitFor(() => dir() === "noseDown");
    await w!.get('[data-orientation="no"]').setValue(true);
    await w!.get('[data-preset="default"]').trigger("click");
    await waitFor(() => dir() !== "noseDown" && dir() !== "");
  });

  it("an unexpected attitude-stopped (ERROR) shows the error with a retry", async () => {
    await open();
    await waitFor(() => mock.device.attitudeTimer !== null);
    mock.events.emit("device://attitude-stopped", { reason: "ERROR", error: { code: "DISCONNECTED", message: "port lost" } });
    await settle();
    expect(w!.find('[data-test="attitude-retry"]').exists()).toBe(true);
    expect(w!.get('[data-test="attitude-busy"]').text()).toContain("stopped answering");
    await w!.get('[data-test="attitude-retry"]').trigger("click");
    await waitFor(() => mock.device.attitudeTimer !== null);
    expect(w!.find('[data-test="attitude-retry"]').exists()).toBe(false);
  });

  it("attitude-stopped DISCONNECTED says the board was disconnected, with a retry", async () => {
    await open();
    await waitFor(() => mock.device.attitudeTimer !== null);
    mock.events.emit("device://attitude-stopped", { reason: "DISCONNECTED", error: null });
    await settle();
    expect(w!.get('[data-test="attitude-busy"]').text()).toContain("disconnected");
    expect(w!.find('[data-test="attitude-retry"]').exists()).toBe(true);
  });

  it("presets start from the board's current alignment; the change takes effect after Apply", async () => {
    await open("happy", (d) => (d.alignment = { roll: 0, pitch: 0, yaw: 270 }));
    await w!.get('[data-orientation="no"]').setValue(true);
    await settle();
    expect(w!.get('[data-preset="yaw270"]').attributes("aria-pressed")).toBe("true");
    expect(w!.get('[data-test="alignment-current"]').text()).toContain("yaw 270°");
    expect(w!.get('[data-test="alignment-after-apply"]').text()).toContain("only after you apply");
    expect(wizard.ext.boardAlignment).toBeNull();
  });

  it("board alignment must be whole degrees -180..360", () => {
    const s = createWizardState();
    s.ext.boardAlignment = { roll: 0, pitch: 0, yaw: 270 };
    expect(guards.calibration!.validate(s)).toEqual([]);
    for (const bad of [{ roll: 0, pitch: 0, yaw: 361 }, { roll: -181, pitch: 0, yaw: 0 }, { roll: 0, pitch: 12.5, yaw: 0 }]) {
      s.ext.boardAlignment = bad;
      expect(guards.calibration!.validate(s).map((i) => i.key)).toEqual(["validation.alignmentInvalid"]);
    }
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
    s.ext.airmode = "ALWAYS";
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
    expect(w!.text()).toContain("אם כבר יש בלוח מתג Prearm, הוא נשאר");
  });
});
