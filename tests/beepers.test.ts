// SPDX-License-Identifier: GPL-3.0-or-later
// Beepers: Betaflight defaults with ON_USB off, the safety beeps stay on, reset, and
// the per-firmware list (full list in the demo).
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { forceDemo, resetDemoState } from "../src/api/fallback";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import { generateCli } from "../src/mocks/scenarios";
import { availableBeaconEvents, availableBeepers, BEEPERS, KEEP_ON, recommendedDisabled, setBeacon, setBeeper } from "../src/wizard/beepers";
import { mockFirmwareCatalog } from "../src/mocks/scenarios";
import { parseCatalog, resetCatalog } from "../src/api/catalog";
import { BEEPER_NAMES } from "../src/types/pendingModel";
import { guards } from "../src/wizard/guards";
import { toPlanModel } from "../src/wizard/modelExt";
import { nav, resetNav } from "../src/wizard/nav";
import { createWizardState, resetWizard, wizard } from "../src/wizard/state";
import { settle } from "./walk";

describe("beeper defaults", () => {
  it("recommended = Betaflight defaults (all on) except ON_USB (#65 default)", () => {
    for (const cat of [null, [], mockFirmwareCatalog("4.5.1").beepers, mockFirmwareCatalog("2025.12.0").beepers]) expect(recommendedDisabled(cat)).toEqual(["ON_USB"]);
    for (const k of KEEP_ON) expect(recommendedDisabled(null)).not.toContain(k);
  });

  it("the list is the #65 BeeperName list (CRASH_FLIP, ARMING_GPS_NO_FIX, MULTI_BEEPS), filtered by the catalog", () => {
    expect(BEEPERS.map((b) => b.name).sort()).toEqual([...BEEPER_NAMES].sort());
    expect(availableBeepers(null)).toEqual(BEEPERS);
    const v2025 = availableBeepers(mockFirmwareCatalog("2025.12.0").beepers).map((b) => b.name);
    expect(v2025).not.toContain("RC_SMOOTHING_INIT_FAIL");
    expect(v2025).toContain("CRASH_FLIP"); // never CRASHFLIP in the model
    expect(v2025).toContain("MULTI_BEEPS");
    expect(availableBeepers(mockFirmwareCatalog("4.5.1").beepers).map((b) => b.name)).toContain("RC_SMOOTHING_INIT_FAIL");
    // Tolerant catalog: {name} objects and unknown names.
    expect(availableBeepers(parseCatalog({ beepers: [{ name: "ON_USB" }, "RX_LOST", "SOMETHING_NEW"] }).beepers).map((b) => b.name)).toEqual(["RX_LOST", "ON_USB"]);
  });

  it("setBeeper toggles by #65 name and keeps display order, only names the firmware has", () => {
    const crash = BEEPERS.find((b) => b.name === "CRASH_FLIP")!;
    const usb = BEEPERS.find((b) => b.name === "ON_USB")!;
    let d = setBeeper(["ON_USB"], crash, false, mockFirmwareCatalog("2025.12.0").beepers);
    expect(d).toEqual(["CRASH_FLIP", "ON_USB"]);
    d = setBeeper(d, usb, true, null);
    expect(d).toEqual(["CRASH_FLIP"]);
    expect(setBeeper(["RC_SMOOTHING_INIT_FAIL"], usb, false, mockFirmwareCatalog("2025.12.0").beepers)).toEqual(["ON_USB"]);
  });

  it("DShot beacon: events from the catalog, toggles keep RX_LOST, RX_SET order", () => {
    expect(availableBeaconEvents(null)).toEqual(["RX_LOST", "RX_SET"]);
    expect(availableBeaconEvents(["RX_SET"])).toEqual(["RX_SET"]);
    expect(setBeacon([], "RX_SET", false)).toEqual(["RX_SET"]);
    expect(setBeacon(["RX_SET"], "RX_LOST", false)).toEqual(["RX_LOST", "RX_SET"]);
    expect(setBeacon(["RX_LOST", "RX_SET"], "RX_LOST", true)).toEqual(["RX_SET"]);
  });

  it("the guard asks for a choice; keep = null, choose = recommended", () => {
    const s = createWizardState();
    expect(guards.beepers!.validate(s).map((i) => i.key)).toEqual(["validation.beepersChoose"]);
    s.ui.beeperMode = "keep";
    expect(guards.beepers!.validate(s)).toEqual([]);
    expect("beepers" in toPlanModel(s.model, s.ext)).toBe(false);
  });

  it("the mock CLI writes beeper lines only for an explicit choice", () => {
    const s = createWizardState();
    expect(generateCli(toPlanModel(s.model, s.ext), []).commands.some((c) => c.startsWith("beeper"))).toBe(false);
    s.ext.beepers = { disabled: ["ON_USB"] };
    const cli = generateCli(toPlanModel(s.model, s.ext), []).commands;
    expect(cli).toContain("beeper -ON_USB");
    expect(cli).toContain("beeper RX_LOST");
    expect(cli.some((c) => c.startsWith("beacon"))).toBe(false);
    s.ext.beepers = { disabled: ["ON_USB"], dshotBeaconDisabled: ["RX_SET"] };
    const cli2 = generateCli(toPlanModel(s.model, s.ext), []).commands;
    expect(cli2).toEqual(expect.arrayContaining(["beacon RX_LOST", "beacon -RX_SET"]));
    expect(toPlanModel(s.model, s.ext).beepers).toEqual({ disabled: ["ON_USB"], dshotBeaconDisabled: ["RX_SET"] });
  });
});

describe("Beepers step UI", () => {
  let w: VueWrapper | null = null;
  beforeEach(() => {
    resetWizard();
    resetNav();
    resetDemoState();
    resetCatalog();
    setLocale("en");
  });
  afterEach(() => {
    w?.unmount();
    w = null;
    uninstallMock();
  });

  it("choose shows grouped checklist, safety beeps locked on, reset to recommended", async () => {
    installMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    forceDemo();
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    nav.currentId = "beepers";
    await settle();
    expect(w.find('[data-group="important"]').exists()).toBe(false);
    await w.get('[data-beepers="choose"]').setValue(true);
    await settle();
    expect(wizard.ext.beepers).toEqual({ disabled: ["ON_USB"] });
    for (const g of ["important", "helpful", "noisy"]) expect(w.find(`[data-group="${g}"]`).exists()).toBe(true);
    // Demo: the full list.
    expect(w.findAll("[data-beeper]").length).toBe(BEEPERS.length);
    for (const k of KEEP_ON) {
      const box = w.get(`[data-beeper="${k}"]`);
      expect((box.element as HTMLInputElement).checked).toBe(true);
      expect(box.attributes("disabled")).toBeDefined();
    }
    expect(w.get('[data-test="keep-on-note"]').text()).toContain("low battery");
    await w.get('[data-beeper="SYSTEM_INIT"]').setValue(false);
    await w.get('[data-beeper="ON_USB"]').setValue(true);
    expect(wizard.ext.beepers).toEqual({ disabled: ["SYSTEM_INIT"] });
    await w.get('[data-test="beepers-reset"]').trigger("click");
    expect(wizard.ext.beepers).toEqual({ disabled: ["ON_USB"] });
    expect(w.find('[data-test="beepers-recommended"]').exists()).toBe(true);
    await w.get('[data-beepers="keep"]').setValue(true);
    expect(wizard.ext.beepers).toBeNull();
  });

  it("a 2025.12 board gets only its beepers, with its names", async () => {
    installMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    wizard.model.fc.firmwareVersion = "2025.12.0";
    nav.currentId = "beepers";
    await settle();
    await w.get('[data-beepers="choose"]').setValue(true);
    await settle();
    expect(w.find('[data-beeper="RC_SMOOTHING_INIT_FAIL"]').exists()).toBe(false);
    expect(w.find('[data-beeper="CRASH_FLIP"]').exists()).toBe(true);
    expect(w.find('[data-beeper="MULTI_BEEPS"]').exists()).toBe(true);
  });

  it("DShot beacon: keep by default, choose sets dshotBeaconDisabled with a plain explanation", async () => {
    installMock({ scenario: "happy", latencyMs: 0, stepMs: 0 });
    forceDemo();
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    wizard.model.esc.protocol = "DSHOT600";
    nav.currentId = "beepers";
    await settle();
    await w.get('[data-beepers="choose"]').setValue(true);
    await settle();
    expect(w.get('[data-section="beacon"]').text()).toContain("motors themselves can beep");
    expect(wizard.ext.beepers?.dshotBeaconDisabled ?? null).toBeNull();
    await w.get('[data-beacon="change"]').setValue(true);
    // Starts from the firmware default: off for both.
    expect(wizard.ext.beepers?.dshotBeaconDisabled).toEqual(["RX_LOST", "RX_SET"]);
    await w.get('[data-beacon-event="RX_LOST"]').setValue(true);
    expect(wizard.ext.beepers?.dshotBeaconDisabled).toEqual(["RX_SET"]);
    // Toggling a beeper keeps the beacon answer.
    await w.get('[data-beeper="SYSTEM_INIT"]').setValue(false);
    expect(wizard.ext.beepers?.dshotBeaconDisabled).toEqual(["RX_SET"]);
    await w.get('[data-beacon="keep"]').setValue(true);
    expect(wizard.ext.beepers?.dshotBeaconDisabled).toBeNull();
    // Not DShot: no beacon section.
    wizard.model.esc.protocol = "ONESHOT125" as never;
    await settle();
    expect(w.find('[data-section="beacon"]').exists()).toBe(false);
  });
});
