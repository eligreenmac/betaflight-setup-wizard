// SPDX-License-Identifier: GPL-3.0-or-later
// OSD step: video system from the VTX (or asked), defaults from the board's default
// layout (enabledByDefault), any grid size, the preview (ticked elements only, 4:3 / 16:9),
// clamping, what is sent (offered elements; x/y only when moved), he/en and RTL.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { resetDemoState } from "../src/api/fallback";
import { parseLayout } from "../src/api/osdPending";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import { generateCli, mockOsdLayout, osdPosValue, type Scenario } from "../src/mocks/scenarios";
import { guards } from "../src/wizard/guards";
import { nav, resetNav } from "../src/wizard/nav";
import { clampPos, emptyOsdState, offeredElements, OSD_ELEMENTS, osdChoice, recommendedEnabled, systemFromVtx, withOsd } from "../src/wizard/osd";
import { createWizardState, resetWizard, wizard } from "../src/wizard/state";
import { settle } from "./walk";

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

type Vtx = typeof wizard.model.vtx;
const ANALOG: Partial<Vtx> = { kind: "ANALOG", protocol: "SMARTAUDIO" };
const DJI: Partial<Vtx> = { kind: "DIGITAL", protocol: "MSP", digitalSystem: "DJI" };

async function open(vtx: Partial<Vtx>, scenario: Scenario = "happy", gps = false): Promise<VueWrapper> {
  installMock({ scenario, latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await w.get('[data-test="next"]').trigger("click");
  await w.get('[data-test="connect"]').trigger("click");
  await settle();
  Object.assign(wizard.model.vtx, vtx);
  wizard.model.gps.enabled = gps;
  nav.currentId = "osd";
  await settle();
  return w;
}
async function choose(): Promise<void> {
  await w!.get('[data-osd-mode="choose"]').setValue(true);
  await settle();
}
const previewNames = () => w!.findAll("[data-preview]").map((e) => e.attributes("data-preview"));

describe("video system", () => {
  it("analog VTX = ANALOG; DJI / Walksnail / HDZero / MSP = HD; otherwise unknown", () => {
    expect(systemFromVtx({ kind: "ANALOG", protocol: "TRAMP" })).toBe("ANALOG");
    for (const d of ["DJI", "WALKSNAIL", "HDZERO"]) expect(systemFromVtx({ kind: "DIGITAL", protocol: null, digitalSystem: d })).toBe("HD");
    expect(systemFromVtx({ kind: "DIGITAL", protocol: "MSP", digitalSystem: "OTHER" })).toBe("HD");
    expect(systemFromVtx({ kind: "DIGITAL", protocol: null, digitalSystem: "OTHER" })).toBeNull();
    expect(systemFromVtx({ kind: "DIGITAL", protocol: null, digitalSystem: null })).toBeNull();
    expect(systemFromVtx({ kind: "NONE" })).toBeNull();
  });

  it("the step is only in the flow with a VTX", () => {
    const s = createWizardState();
    expect(guards.osd).toBeDefined();
    s.model.vtx.kind = "NONE";
    expect(guards.osd.validate(s).map((i) => i.key)).toEqual(["validation.osdSystem"]);
  });

  it("unknown system: asks with simple descriptions, then loads the matching layout", async () => {
    await open({ kind: "DIGITAL", protocol: null, digitalSystem: "OTHER" });
    expect(w!.text()).toContain("couldn't tell which video system");
    expect(w!.text()).toContain("Classic FPV video");
    expect(w!.text()).toContain("DJI O3/O4, Walksnail Avatar or HDZero");
    expect(guards.osd.validate(wizard).map((i) => i.key)).toEqual(["validation.osdSystem"]);
    await w!.get('[data-osd-system="HD"]').setValue(true);
    await settle();
    expect(wizard.osd.layoutSystem).toBe("HD");
    expect(guards.osd.validate(wizard).map((i) => i.key)).toEqual(["validation.osdChoose"]);
  });

  it("detected system can be corrected", async () => {
    await open(ANALOG);
    expect(w!.get('[data-test="osd-detected"]').text()).toContain("analog");
    await w!.get('[data-test="osd-change-system"]').trigger("click");
    await w!.get('[data-osd-system="HD"]').setValue(true);
    await settle();
    expect(wizard.osd.layout?.grid).toEqual({ cols: 53, rows: 20 });
  });
});

describe("layout and defaults", () => {
  it("parseLayout tolerates extra keys and drops out-of-grid / duplicate elements", () => {
    const l = parseLayout({
      grid: { cols: 30, rows: 13, extra: 1 },
      elements: [
        { name: "vbat", x: 1, y: 12, enabledByDefault: true, more: "x" },
        { name: "vbat", x: 2, y: 2, enabledByDefault: false },
        { name: "rssi", x: 30, y: 1, enabledByDefault: true },
        { name: "tim_2", x: 3, y: 13, enabledByDefault: true },
      ],
      future: true,
    });
    expect(l).toEqual({ grid: { cols: 30, rows: 13 }, elements: [{ name: "vbat", x: 1, y: 12, enabledByDefault: true }] });
  });

  it("defaults come from enabledByDefault (no hard-coded set); GPS items only with GPS", () => {
    const l = mockOsdLayout("ANALOG");
    const rec = recommendedEnabled(l, false);
    expect(rec.avg_cell_voltage).toBe(true);
    expect(rec.vbat).toBe(false);
    expect("gps_sats" in rec).toBe(false);
    expect("pid_roll" in rec).toBe(false); // not offered by the UI
    expect(recommendedEnabled(l, true).gps_sats).toBe(true);
    const flipped = { ...l, elements: l.elements.map((e) => ({ ...e, enabledByDefault: !e.enabledByDefault })) };
    expect(recommendedEnabled(flipped, false).vbat).toBe(true);
    expect(recommendedEnabled(flipped, false).avg_cell_voltage).toBe(false);
  });

  it("only elements in the board's layout are offered", () => {
    const l = { grid: { cols: 30, rows: 16 }, elements: [{ name: "vbat", x: 1, y: 1, enabledByDefault: true }] };
    expect(offeredElements(l, true).map((e) => e.name)).toEqual(["vbat"]);
    expect(offeredElements(null, true)).toEqual([]);
  });

  it("every UI element name is a Betaflight osd_<name>_pos stem", () => {
    for (const e of OSD_ELEMENTS) expect(e.name).toMatch(/^[a-z0-9_]+$/);
    expect(OSD_ELEMENTS.map((e) => e.name)).toContain("avg_cell_voltage");
  });
});

describe("preview", () => {
  it("analog: 4:3 canvas on the PAL 30x16 grid, ticked elements only, with sample values", async () => {
    await open(ANALOG);
    await choose();
    const p = w!.get('[data-test="osd-preview"]');
    expect(p.classes()).toContain("osd-preview--analog");
    expect(p.attributes("dir")).toBe("ltr");
    expect([p.attributes("data-cols"), p.attributes("data-rows")]).toEqual(["30", "16"]);
    const on = Object.entries(wizard.osd.enabled).filter(([, v]) => v).map(([k]) => k);
    expect(previewNames().sort()).toEqual(on.sort());
    expect(w!.get('[data-preview="avg_cell_voltage"]').text()).toBe("4.12V");
    // Position from the layout: x=1, y=rows-2 → left 1/30, top 14/16.
    const style = w!.get('[data-preview="avg_cell_voltage"]').attributes("style");
    expect(style).toContain(`left: ${(1 / 30) * 100}%`);
    expect(style).toContain(`top: ${(14 / 16) * 100}%`);
    // Untick → gone; tick an optional one → shown.
    await w!.get('[data-osd="avg_cell_voltage"]').setValue(false);
    await w!.get('[data-osd="throttle"]').setValue(true);
    await settle();
    expect(previewNames()).not.toContain("avg_cell_voltage");
    expect(previewNames()).toContain("throttle");
  });

  it("HD: 16:9 canvas on the HD grid", async () => {
    await open(DJI);
    await choose();
    const p = w!.get('[data-test="osd-preview"]');
    expect(p.classes()).toContain("osd-preview--hd");
    expect([p.attributes("data-cols"), p.attributes("data-rows")]).toEqual(["53", "20"]);
  });

  it("any grid: NTSC 30x13", async () => {
    await open(ANALOG, "osd-ntsc");
    await choose();
    const p = w!.get('[data-test="osd-preview"]');
    expect([p.attributes("data-cols"), p.attributes("data-rows")]).toEqual(["30", "13"]);
    const style = w!.get('[data-preview="mah_drawn"]').attributes("style");
    expect(style).toContain(`top: ${(11 / 13) * 100}%`);
  });

  it("arrow keys move an element, clamped to the grid, and mark it moved", async () => {
    await open(ANALOG);
    await choose();
    const el = w!.get('[data-preview="link_quality"]'); // at (1,1)
    for (let i = 0; i < 4; i++) await el.trigger("keydown", { key: "ArrowLeft" });
    await el.trigger("keydown", { key: "ArrowUp" });
    await el.trigger("keydown", { key: "ArrowUp" });
    await settle();
    expect(wizard.osd.moved.link_quality).toEqual({ x: 0, y: 0 });
    expect(w!.get('[data-test="osd-reset"]').attributes("disabled")).toBeUndefined();
    await w!.get('[data-test="osd-reset"]').trigger("click");
    await settle();
    expect(wizard.osd.moved).toEqual({});
  });

  it("Reset to recommended restores enabledByDefault", async () => {
    await open(ANALOG);
    await choose();
    const before = { ...wizard.osd.enabled };
    await w!.get('[data-osd="vbat"]').setValue(true);
    await settle();
    expect(w!.find('[data-test="osd-recommended"]').exists()).toBe(false);
    await w!.get('[data-test="osd-reset"]').trigger("click");
    await settle();
    expect(wizard.osd.enabled).toEqual(before);
    expect(w!.find('[data-test="osd-recommended"]').exists()).toBe(true);
  });
});

describe("what is sent", () => {
  it("clampPos keeps the whole element on any grid", () => {
    expect(clampPos(-3, -1, { cols: 30, rows: 16 }, 5)).toEqual({ x: 0, y: 0 });
    expect(clampPos(29, 20, { cols: 30, rows: 16 }, 5)).toEqual({ x: 25, y: 15 });
    expect(clampPos(29, 20, { cols: 30, rows: 13 }, 5)).toEqual({ x: 25, y: 12 });
    expect(clampPos(60, 3, { cols: 53, rows: 20 }, 99)).toEqual({ x: 0, y: 3 });
    expect(clampPos(2.6, 3.4, { cols: 53, rows: 20 })).toEqual({ x: 3, y: 3 });
  });

  it("keep / untouched = no osd field (keep the board's OSD)", () => {
    const s = emptyOsdState();
    expect(osdChoice(s, false)).toBeNull();
    s.mode = "keep";
    s.layout = mockOsdLayout("ANALOG");
    s.layoutSystem = "ANALOG";
    expect(osdChoice(s, false)).toBeNull();
    expect("osd" in withOsd(createWizardState().model, s)).toBe(false);
  });

  it("choose: offered elements with enabled; x/y only for moved ones (clamped)", () => {
    const s = emptyOsdState();
    s.mode = "choose";
    s.layout = mockOsdLayout("ANALOG", "NTSC");
    s.layoutSystem = "ANALOG";
    s.enabled = recommendedEnabled(s.layout, false);
    s.moved = { vbat: { x: 99, y: 99 } };
    const c = osdChoice(s, false)!;
    expect(c.system).toBe("ANALOG");
    expect(c.elements.map((e) => e.name)).not.toContain("pid_roll");
    expect(c.elements.map((e) => e.name)).not.toContain("gps_sats");
    expect(c.elements.find((e) => e.name === "vbat")).toEqual({ name: "vbat", enabled: false, x: 30 - 5, y: 12 });
    expect(c.elements.find((e) => e.name === "avg_cell_voltage")).toEqual({ name: "avg_cell_voltage", enabled: true });
  });

  it("the mock plan writes osd_<name>_pos with BF's position encoding", () => {
    expect(osdPosValue(1, 14, true)).toBe(1 | (14 << 5) | 0x800);
    expect(osdPosValue(40, 3, false)).toBe((40 & 31) | (1 << 10) | (3 << 5));
    const s = createWizardState();
    const osd = { system: "HD" as const, elements: [{ name: "vbat", enabled: true, x: 40, y: 3 }, { name: "tim_2", enabled: false }] };
    const cli = generateCli({ ...s.model, osd }, []);
    expect(cli.commands).toContain(`set osd_vbat_pos = ${osdPosValue(40, 3, true)}`);
    expect(cli.commands).toContain(`set osd_tim_2_pos = ${osdPosValue(53 - 7, 1, false)}`);
    expect(cli.commands.some((c) => c.startsWith("set osd_") && c.includes("rssi"))).toBe(false);
    expect(generateCli(s.model, []).commands.some((c) => c.startsWith("set osd_"))).toBe(false);
  });

  it("review shows the OSD row", async () => {
    await open(ANALOG);
    await choose();
    nav.currentId = "review";
    await settle();
    expect(w!.text()).toMatch(/OSD\s*\d+ items shown \(Analog\)/);
    expect(w!.find('[data-test="osd-unplanned"]').exists()).toBe(false); // the mock plan writes it
  });
});

describe("Hebrew", () => {
  it("renders in Hebrew; the page is RTL but the preview stays LTR", async () => {
    await open(DJI);
    setLocale("he");
    await choose();
    expect(document.documentElement.dir).toBe("rtl");
    expect(w!.text()).toContain("תצוגה על המסך");
    expect(w!.text()).toContain("חזרה להמלצה");
    expect(w!.text()).toContain("מתח סוללה לתא");
    expect(w!.get('[data-test="osd-preview"]').attributes("dir")).toBe("ltr");
  });
});
