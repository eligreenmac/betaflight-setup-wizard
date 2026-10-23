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
import { availableBeepers, BEEPERS, beeperName, KEEP_ON, recommendedDisabled, setBeeper } from "../src/wizard/beepers";
import { guards } from "../src/wizard/guards";
import { toPlanModel } from "../src/wizard/modelExt";
import { nav, resetNav } from "../src/wizard/nav";
import { createWizardState, resetWizard, wizard } from "../src/wizard/state";
import { settle } from "./walk";

describe("beeper defaults", () => {
  it("recommended = Betaflight defaults (all on) except ON_USB", () => {
    for (const fw of [null, "4.4.3", "4.5.1", "2025.12.0"]) expect(recommendedDisabled(fw)).toEqual(["ON_USB"]);
    for (const k of KEEP_ON) expect(recommendedDisabled(null)).not.toContain(k);
  });

  it("per firmware: 2025.12 renames CRASH_FLIP and drops RC_SMOOTHING_INIT_FAIL; the demo shows all", () => {
    const crash = BEEPERS.find((b) => b.key === "CRASH_FLIP")!;
    expect(beeperName(crash, "4.5.1")).toBe("CRASH_FLIP");
    expect(beeperName(crash, "2025.12.0")).toBe("CRASHFLIP");
    expect(availableBeepers("2025.12.0").map((b) => b.key)).not.toContain("RC_SMOOTHING_INIT_FAIL");
    expect(availableBeepers("4.5.1").map((b) => b.key)).toContain("RC_SMOOTHING_INIT_FAIL");
    expect(availableBeepers(null)).toEqual(BEEPERS);
  });

  it("setBeeper toggles by firmware name and keeps firmware order", () => {
    const crash = BEEPERS.find((b) => b.key === "CRASH_FLIP")!;
    const usb = BEEPERS.find((b) => b.key === "ON_USB")!;
    let d = setBeeper(["ON_USB"], crash, false, "2025.12.0");
    expect(d).toEqual(["CRASHFLIP", "ON_USB"]);
    d = setBeeper(d, usb, true, "2025.12.0");
    expect(d).toEqual(["CRASHFLIP"]);
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
  });
});

describe("Beepers step UI", () => {
  let w: VueWrapper | null = null;
  beforeEach(() => {
    resetWizard();
    resetNav();
    resetDemoState();
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
    expect(w.text()).toContain("CRASHFLIP");
  });
});
