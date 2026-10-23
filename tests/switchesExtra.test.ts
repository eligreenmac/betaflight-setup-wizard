// SPDX-License-Identifier: GPL-3.0-or-later
// Switches step additions: PREARM (optional, explicit skip), turtle (DShot), AIRMODE
// (always / switch / off / keep), live "Detect", and validation against ARM.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import type { ScenarioMock } from "../src/mocks/scenarios";
import { guards } from "../src/wizard/guards";
import { nav, resetNav } from "../src/wizard/nav";
import { toPlanModel } from "../src/wizard/modelExt";
import { generateCli } from "../src/mocks/scenarios";
import { addModeSwitch, createWizardState, removeModeSwitch, resetWizard, setAirmode, setModeRemoval, setPrearm, wizard, type WizardState } from "../src/wizard/state";
import { unplannedChoices } from "../src/wizard/modelExt";
import type { DroneModelNext } from "../src/types/pendingModel";
import { detectSwitch, MODES, modeKeyOf, SWITCH_POSITIONS } from "../src/wizard/switches";
import { settle } from "./walk";

const keys = (s: WizardState) => guards.switches!.validate(s).map((i) => i.key);
const modes = (s: WizardState) => s.model.switches.map((x) => x.mode);

describe("switch validation", () => {
  it("PREARM is optional: no answer and an explicit skip are both valid", () => {
    const s = createWizardState();
    expect(keys(s)).toEqual([]);
    setPrearm(s, "skip");
    expect(s.ui.prearmChoice).toBe("skip");
    expect(modes(s)).not.toContain(MODES.prearm);
    expect(keys(s)).toEqual([]);
  });

  it("PREARM added goes on its own AUX channel; skipping again removes it", () => {
    const s = createWizardState();
    setPrearm(s, "add");
    expect(s.ui.prearmChoice).toBe("add");
    const p = s.model.switches.find((x) => x.mode === "PREARM")!;
    expect(p.auxChannel).not.toBe(s.model.switches[0]!.auxChannel);
    expect(keys(s)).toEqual([]);
    setPrearm(s, "skip");
    expect(modes(s)).toEqual(["ARM"]);
  });

  it("PREARM on the ARM switch channel is refused", () => {
    const s = createWizardState();
    setPrearm(s, "add");
    const p = s.model.switches.find((x) => x.mode === "PREARM")!;
    p.auxChannel = 0;
    p.rangeMin = SWITCH_POSITIONS.mid[0];
    p.rangeMax = SWITCH_POSITIONS.mid[1];
    expect(keys(s)).toEqual(["validation.prearmSameSwitch"]);
  });

  it("any mode overlapping the ARM range is refused (incl. the new modes)", () => {
    const s = createWizardState();
    setAirmode(s, "SWITCH");
    const a = s.model.switches.find((x) => x.mode === "AIR MODE")!;
    a.auxChannel = 0;
    expect(keys(s)).toContain("validation.armOverlap");
  });

  it("turtle mode needs DShot", () => {
    const s = createWizardState();
    addModeSwitch(s, "turtle");
    s.model.esc.protocol = "DSHOT600";
    expect(keys(s)).toEqual([]);
    s.model.esc.protocol = "ONESHOT125" as never;
    expect(keys(s)).toEqual(["validation.turtleNeedsDshot"]);
  });

  it("AIRMODE: keep (null) by default, always/off set the field, switch adds the AIR MODE box", () => {
    const s = createWizardState();
    expect(s.ext.airmode).toBeNull();
    expect("airmode" in toPlanModel(s.model, s.ext)).toBe(false);
    setAirmode(s, "ALWAYS");
    expect(toPlanModel(s.model, s.ext).airmode).toBe("ALWAYS");
    expect(modes(s)).toEqual(["ARM"]);
    setAirmode(s, "SWITCH");
    expect(modes(s)).toEqual(["ARM", "AIR MODE"]);
    expect(keys(s)).toEqual([]);
    setAirmode(s, "OFF");
    expect(modes(s)).toEqual(["ARM"]);
    expect(toPlanModel(s.model, s.ext).airmode).toBe("OFF");
    setAirmode(s, null);
    expect("airmode" in toPlanModel(s.model, s.ext)).toBe(false);
  });

  it("removing the AIR MODE row goes back to 'keep'; a missing switch is reported", () => {
    const s = createWizardState();
    setAirmode(s, "SWITCH");
    removeModeSwitch(s, s.model.switches.findIndex((x) => x.mode === "AIR MODE"));
    expect(s.ext.airmode).toBeNull();
    s.ext.airmode = "SWITCH";
    expect(keys(s)).toEqual(["validation.airmodeSwitchMissing"]);
  });

  it("the mock CLI maps the spaced and squashed names to box ids", () => {
    const s = createWizardState();
    setPrearm(s, "add");
    setAirmode(s, "SWITCH");
    addModeSwitch(s, "turtle");
    const cli = generateCli(toPlanModel(s.model, s.ext), []).commands.filter((c) => c.startsWith("aux "));
    expect(cli.map((c) => c.split(" ")[2])).toEqual(["0", "36", "28", "35"]);
    expect(modeKeyOf("AIRMODE")).toBe("airMode");
    expect(modeKeyOf("FLIP_OVER_AFTER_CRASH")).toBe("turtle");
  });

  it("PREARM: skip = keep (null, nothing sent); remove = removeModes; add drops the removal", () => {
    const s = createWizardState();
    setPrearm(s, "skip");
    let p = toPlanModel(s.model, s.ext);
    expect("removeModes" in p).toBe(false);
    expect(modes(s)).toEqual(["ARM"]);
    setPrearm(s, "remove");
    expect(s.ui.prearmChoice).toBe("remove");
    p = toPlanModel(s.model, s.ext);
    expect(p.removeModes).toEqual(["PREARM"]);
    expect(modes(s)).toEqual(["ARM"]);
    setPrearm(s, "add");
    expect(s.ext.removeModes).toEqual([]);
    expect(modes(s)).toEqual(["ARM", "PREARM"]);
    setPrearm(s, "remove");
    expect(modes(s)).toEqual(["ARM"]);
  });

  it("turtle / AIR MODE removal: removing drops the plan's switch; airmode SWITCH goes back to keep", () => {
    const s = createWizardState();
    addModeSwitch(s, "turtle");
    setModeRemoval(s, "FLIP OVER AFTER CRASH", true);
    expect(modes(s)).toEqual(["ARM"]);
    setAirmode(s, "SWITCH");
    setModeRemoval(s, "AIR MODE", true);
    expect(s.ext.airmode).toBeNull();
    expect(toPlanModel(s.model, s.ext).removeModes).toEqual(["FLIP OVER AFTER CRASH", "AIR MODE"]);
    // Adding the switch again cancels its removal.
    addModeSwitch(s, "turtle");
    expect(s.ext.removeModes).toEqual(["AIR MODE"]);
    setModeRemoval(s, "AIR MODE", false);
    expect("removeModes" in toPlanModel(s.model, s.ext)).toBe(false);
    // The mock plan writes a reset line for a removal (a no-op on the real engine if the FC has none).
    setModeRemoval(s, "PREARM", true);
    const p = toPlanModel(s.model, s.ext);
    expect(generateCli(p, []).commands.filter((c) => c.startsWith("aux ")).length).toBe(p.switches.length + 1);
    expect(unplannedChoices(s.ext, generateCli(p, []))).toEqual([]);
  });

  it("the plan model has the exact #65 shapes", () => {
    const s = createWizardState();
    setAirmode(s, "SWITCH");
    setPrearm(s, "remove");
    s.ext.sensors = { acc: null, baro: false, mag: null };
    s.ext.beepers = { disabled: ["ON_USB"], dshotBeaconDisabled: null };
    s.ext.boardAlignment = { roll: 0, pitch: 0, yaw: 90 };
    const p: DroneModelNext = toPlanModel(s.model, s.ext);
    expect(p.airmode).toBe("SWITCH");
    expect(p.switches.map((x) => x.mode)).toEqual(["ARM", "AIR MODE"]);
    expect(p.removeModes).toEqual(["PREARM"]);
    expect(p.sensors).toEqual({ baro: false });
    expect(p.beepers).toEqual({ disabled: ["ON_USB"], dshotBeaconDisabled: null });
    expect(p.boardAlignment).toEqual({ roll: 0, pitch: 0, yaw: 90 });
  });

  it("detectSwitch picks the AUX channel that moved most, with its position", () => {
    const base = [1500, 1500, 1000, 1500, 1000, 1000, 1000, 1000];
    expect(detectSwitch(base, base)).toBeNull();
    expect(detectSwitch(base, [1500, 1500, 1000, 1500, 1100, 1000, 1000, 1000])).toBeNull(); // too small
    expect(detectSwitch(base, [1500, 1500, 1000, 1500, 1000, 1000, 2000, 1000])).toEqual({ auxChannel: 2, position: "high" });
    expect(detectSwitch(base, [1500, 1500, 1000, 1500, 1000, 1500, 1000, 1000])).toEqual({ auxChannel: 1, position: "mid" });
    // Sticks (channels 1-4) never count.
    expect(detectSwitch(base, [2000, 1000, 2000, 1000, 1000, 1000, 1000, 1000])).toBeNull();
  });
});

describe("Switches step UI", () => {
  let mock: ScenarioMock;
  let w: VueWrapper | null = null;
  beforeEach(() => {
    resetWizard();
    resetNav();
    setLocale("en");
  });
  afterEach(() => {
    w?.unmount();
    w = null;
    uninstallMock();
  });
  async function open(): Promise<VueWrapper> {
    mock = installMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    wizard.connectedPath = "COM5";
    nav.currentId = "switches";
    await settle();
    return w;
  }

  it("PREARM skip, turtle and air mode controls change the model", async () => {
    await open();
    expect(w!.get('[data-section="prearm"]').text()).toContain("If the board already has a prearm switch, it stays");
    await w!.get('[data-prearm="skip"]').setValue(true);
    expect(wizard.ui.prearmChoice).toBe("skip");
    await w!.get('[data-prearm="add"]').setValue(true);
    expect(wizard.model.switches.map((s) => s.mode)).toContain("PREARM");
    await w!.get('[data-add="turtle"]').trigger("click");
    expect(wizard.model.switches.map((s) => s.mode)).toContain("FLIP OVER AFTER CRASH");
    await w!.get('[data-airmode="switch"]').setValue(true);
    expect(wizard.ext.airmode).toBe("SWITCH");
    expect(wizard.model.switches.map((s) => s.mode)).toContain("AIR MODE");
    await w!.get('[data-airmode="always"]').setValue(true);
    expect(wizard.ext.airmode).toBe("ALWAYS");
    expect(wizard.model.switches.map((s) => s.mode)).not.toContain("AIR MODE");
    await w!.get('[data-prearm="remove"]').setValue(true);
    expect(wizard.ext.removeModes).toEqual(["PREARM"]);
    expect(wizard.model.switches.map((s) => s.mode)).not.toContain("PREARM");
    // "Remove an existing AIR MODE switch" is offered when air mode is not on a switch.
    await w!.get('[data-remove-mode="airMode"]').setValue(true);
    expect(wizard.ext.removeModes).toEqual(["PREARM", "AIR MODE"]);
    expect(w!.find('[data-remove-mode="turtle"]').exists()).toBe(false); // turtle is on a switch now
    // Mode availability from the catalog: 4.5 has no ALTHOLD; GPS modes only with GPS.
    const adds = w!.findAll("[data-add]").map((b) => b.attributes("data-add"));
    expect(adds).toContain("horizon");
    expect(adds).not.toContain("altHold");
    expect(adds).not.toContain("gpsRescue");
    // Generic add buttons do not duplicate the explained modes.
    expect(w!.findAll("[data-add]").map((b) => b.attributes("data-add"))).not.toContain("prearm");
  });

  it("Detect assigns the AUX channel the user flips", async () => {
    await open();
    await settle();
    await w!.get('[data-prearm="add"]').setValue(true);
    await settle();
    const row = wizard.model.switches.findIndex((s) => s.mode === "PREARM");
    // Stop the mock's own RC stream so only our frames arrive.
    await mock.backend("device_rc_stream_stop" as never, {} as never);
    const base = [1500, 1500, 1000, 1500, 1000, 1000, 1000, 1000, 1000];
    mock.events.emit("device://rc-channels", { seq: 1, timestampMs: 0, channels: base });
    await settle();
    await w!.get(`[data-detect="${row}"]`).trigger("click");
    expect(w!.find('[data-test="detect-hint"]').exists()).toBe(true);
    const moved = [...base];
    moved[8] = 1500; // AUX5 to the middle
    mock.events.emit("device://rc-channels", { seq: 2, timestampMs: 100, channels: moved });
    await settle();
    expect(wizard.model.switches[row]).toMatchObject({ auxChannel: 4, rangeMin: SWITCH_POSITIONS.mid[0], rangeMax: SWITCH_POSITIONS.mid[1] });
    expect(w!.find('[data-test="detect-hint"]').exists()).toBe(false);
  });
});
