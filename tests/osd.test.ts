// SPDX-License-Identifier: GPL-3.0-or-later
// OSD step (#65 `osd`, #66 layout / catalog): video system from the VTX (analog = AUTO,
// PAL / NTSC on request; HD gated on the firmware's build options), defaults from
// config_osd_default_layout (`enabled`), any grid size, the preview (ticked elements only,
// 4:3 / 16:9), clamping, what is sent (offered elements; x/y only when moved), he/en and RTL.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../src/api/commands";
import App from "../src/App.vue";
import { resetDemoState } from "../src/api/fallback";
import { parseLayout } from "../src/api/osdLayout";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import { generateCli, mockFirmwareCatalog, mockOsdLayout, osdPosValue, type Scenario } from "../src/mocks/scenarios";
import { guards } from "../src/wizard/guards";
import { nav, resetNav } from "../src/wizard/nav";
import { analogOnDigital, clampPos, effectiveSystem, emptyOsdState, hdOsdSupport, isHdOsdPlanError, offeredElements, OSD_ELEMENTS, osdChoice, recommendedEnabled, systemFromVtx, withOsd } from "../src/wizard/osd";
import { resetCatalog } from "../src/api/catalog";
import { createWizardState, resetWizard, setBoard, wizard } from "../src/wizard/state";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { settle } from "./walk";

let w: VueWrapper | null = null;

beforeEach(() => {
  resetWizard();
  resetNav();
  resetDemoState();
  resetCatalog();
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
const fcHd = () => wizard.fc?.hdOsdSupported;
const previewNames = () => w!.findAll("[data-preview]").map((e) => e.attributes("data-preview"));

describe("video system", () => {
  it("analog VTX = AUTO; DJI / Walksnail / HDZero / MSP = HD; otherwise unknown", () => {
    expect(systemFromVtx({ kind: "ANALOG", protocol: "TRAMP" })).toBe("AUTO");
    for (const d of ["DJI", "WALKSNAIL", "HDZERO"]) expect(systemFromVtx({ kind: "DIGITAL", protocol: null, digitalSystem: d })).toBe("HD");
    expect(systemFromVtx({ kind: "DIGITAL", protocol: "MSP", digitalSystem: "OTHER" })).toBe("HD");
    expect(systemFromVtx({ kind: "DIGITAL", protocol: null, digitalSystem: "OTHER" })).toBeNull();
    expect(systemFromVtx({ kind: "DIGITAL", protocol: null, digitalSystem: null })).toBeNull();
    expect(systemFromVtx({ kind: "NONE" })).toBeNull();
  });

  it("the step needs a video system", () => {
    const s = createWizardState();
    expect(guards.osd).toBeDefined();
    s.model.vtx.kind = "NONE";
    expect(guards.osd.validate(s).map((i) => i.key)).toEqual(["validation.osdSystem"]);
  });

  it("unknown system: asks (Auto / PAL / NTSC / HD, simple descriptions), then loads the matching layout", async () => {
    await open({ kind: "DIGITAL", protocol: null, digitalSystem: "OTHER" });
    expect(w!.text()).toContain("couldn't tell which video system");
    expect(w!.text()).toContain("Pick this if you're not sure");
    expect(w!.text()).toContain("DJI O3/O4, Walksnail Avatar or HDZero");
    expect(w!.findAll("[data-osd-system]").map((e) => e.attributes("data-osd-system"))).toEqual(["AUTO", "PAL", "NTSC", "HD"]);
    expect(guards.osd.validate(wizard).map((i) => i.key)).toEqual(["validation.osdSystem"]);
    await w!.get('[data-osd-system="HD"]').setValue(true);
    await settle();
    expect(wizard.osd.layout?.videoSystem).toBe("HD");
    expect(guards.osd.validate(wizard).map((i) => i.key)).toEqual(["validation.osdChoose"]);
  });

  it("analog is detected as Auto with a short explanation, and can be corrected to PAL", async () => {
    await open(ANALOG);
    expect(w!.get('[data-test="osd-detected"]').text()).toContain("matches your camera (PAL or NTSC)");
    expect(wizard.osd.layout).toMatchObject({ videoSystem: "AUTO", cols: 30, rows: 13 });
    await w!.get('[data-test="osd-change-system"]').trigger("click");
    await w!.get('[data-osd-system="PAL"]').setValue(true);
    await settle();
    expect(wizard.osd.layout).toMatchObject({ videoSystem: "PAL", cols: 30, rows: 16 });
  });

  it("the layout is requested for the chosen video system and the board's firmware", async () => {
    await open(ANALOG, "fw-2025");
    expect(wizard.osd.layoutKey).toBe(`AUTO@${wizard.model.fc.firmwareVersion}`);
    expect(wizard.model.fc.firmwareVersion.startsWith("2025.12")).toBe(true);
    // 2025.12-only elements are in that layout (none of them offered by the UI).
    expect(wizard.osd.layout?.elements.map((e) => e.name)).toContain("DEBUG2");
  });
});

describe("HD gating (FcInfo.hdOsdSupported, else build options)", () => {
  it("hdOsdSupport: HD option = yes; OSD options without HD = no; nothing about the OSD = unknown", () => {
    expect(hdOsdSupport({ buildOptions: ["USE_OSD_HD", "USE_OSD_SD"] })).toBe(true);
    expect(hdOsdSupport({ buildOptions: ["use_osd_hd"] })).toBe(true);
    expect(hdOsdSupport({ buildOptions: ["USE_OSD_SD", "USE_SERIALRX_CRSF"] })).toBe(false);
    expect(hdOsdSupport({ buildOptions: ["USE_OSD"] })).toBe(false);
    expect(hdOsdSupport({ buildOptions: ["USE_OSD_HD", "USE_MSP_DISPLAYPORT"] })).toBe(true);
    expect(hdOsdSupport({ buildOptions: ["USE_SERIALRX_CRSF", "USE_GPS"] })).toBeNull();
    expect(hdOsdSupport({ buildOptions: [] })).toBeNull();
    expect(hdOsdSupport(null)).toBeNull();
  });

  it("hdOsdSupport: USE_MSP_DISPLAYPORT alone is not HD", () => {
    expect(hdOsdSupport({ buildOptions: ["USE_MSP_DISPLAYPORT"] })).toBe(false);
    expect(hdOsdSupport({ buildOptions: ["use_msp_displayport", "USE_DSHOT"] })).toBe(false);
    expect(hdOsdSupport({ buildOptions: ["USE_MSP_DISPLAYPORT"], hdOsdSupported: null })).toBe(false);
  });

  it("hdOsdSupport on a realistic 4.5 build-options list, with and without USE_OSD_HD", () => {
    // What a 4.5.2 cloud build of SPEEDYBEEF405V4 with the Configurator's default options
    // reports via MSP_BUILD_INFO (Build API /api/options/4.5.2 defaults). The composed
    // speedybeef405v4 4.5 dump fixture is a SITL dump and carries no build options.
    const v45 = [
      "USE_SERIALRX_CRSF",
      "USE_DSHOT",
      "USE_ACRO_TRAINER",
      "USE_GPS",
      "USE_LED_STRIP",
      "USE_OPTICALFLOW",
      "USE_OSD_SD",
      "USE_OSD_HD",
      "USE_PINIO",
      "USE_RANGEFINDER",
      "USE_VTX",
    ];
    expect(hdOsdSupport({ buildOptions: v45, hdOsdSupported: null })).toBe(true);
    const noHd = v45.filter((o) => o !== "USE_OSD_HD");
    expect(hdOsdSupport({ buildOptions: noHd, hdOsdSupported: null })).toBe(false);
    expect(hdOsdSupport({ buildOptions: [...noHd, "USE_MSP_DISPLAYPORT"], hdOsdSupported: null })).toBe(false);
    const noOsd = noHd.filter((o) => o !== "USE_OSD_SD");
    expect(hdOsdSupport({ buildOptions: noOsd, hdOsdSupported: null })).toBeNull();
  });

  it("hdOsdSupport: FcInfo.hdOsdSupported wins when it is a boolean; null / missing / junk = build options", () => {
    expect(hdOsdSupport({ buildOptions: ["USE_OSD_SD"], hdOsdSupported: true })).toBe(true);
    expect(hdOsdSupport({ buildOptions: ["USE_OSD_HD"], hdOsdSupported: false })).toBe(false);
    expect(hdOsdSupport({ buildOptions: [], hdOsdSupported: null })).toBeNull();
    expect(hdOsdSupport({ buildOptions: ["USE_OSD_SD"], hdOsdSupported: null })).toBe(false);
    expect(hdOsdSupport({ buildOptions: ["CRSF"] })).toBeNull();
    expect(hdOsdSupport({ buildOptions: ["USE_OSD_HD"], hdOsdSupported: "yes" } as never)).toBe(true);
    expect(hdOsdSupport({ buildOptions: null } as never)).toBeNull();
  });

  it("supported (true): HD detected and used, no note", async () => {
    await open(DJI);
    expect(fcHd()).toBe(true);
    expect(w!.get('[data-test="osd-detected"]').text()).toContain("digital HD");
    expect(wizard.osd.layout?.videoSystem).toBe("HD");
    expect(w!.find('[data-test="osd-hd-unknown"]').exists()).toBe(false);
    expect(w!.find('[data-test="osd-hd-unsupported"]').exists()).toBe(false);
  });

  it("unknown (null): HD allowed, with a soft note", async () => {
    await open(DJI, "osd-hd-unknown");
    expect(fcHd()).toBeNull();
    expect(effectiveSystem(wizard.model, wizard.osd, wizard.fc)).toBe("HD");
    expect(wizard.osd.layout?.videoSystem).toBe("HD");
    const note = w!.get('[data-test="osd-hd-unknown"]');
    expect(note.classes()).not.toContain("callout--warn");
    expect(note.text()).toContain("couldn't confirm");
    await w!.get('[data-test="osd-change-system"]').trigger("click");
    await settle();
    expect(w!.get('[data-osd-system="HD"]').attributes("disabled")).toBeUndefined();
    await choose();
    expect(guards.osd.validate(wizard)).toEqual([]);
  });

  it("unsupported (false): HD disabled with a plain explanation; analog is not affected", async () => {
    await open(ANALOG, "osd-hd-unsupported");
    expect(fcHd()).toBe(false);
    expect(w!.find('[data-test="osd-hd-blocked"]').exists()).toBe(false);
    await w!.get('[data-test="osd-change-system"]').trigger("click");
    await settle();
    expect(w!.get('[data-osd-system="HD"]').attributes("disabled")).toBeDefined();
    expect(w!.get('[data-test="osd-hd-unsupported"]').text()).toBe(en.osd.system.hdUnsupported);
    await w!.get('[data-osd-system="HD"]').setValue(true);
    await settle();
    expect(wizard.osd.askedSystem).not.toBe("HD");
    expect(w!.find('[data-test="osd-hd-unknown"]').exists()).toBe(false);
  });

  it("unsupported + detected HD VTX: a clear warning and a choice (update firmware / keep current / analog)", async () => {
    await open(DJI, "osd-hd-unsupported");
    expect(effectiveSystem(wizard.model, wizard.osd, wizard.fc)).toBeNull();
    const box = w!.get('[data-test="osd-hd-blocked"]');
    expect(box.attributes("role")).toBe("alert");
    expect(box.text()).toContain(en.osd.system.hdBlocked);
    expect(box.text()).toContain(en.osd.system.hdChoices);
    expect(guards.osd.validate(wizard).map((i) => i.key)).toEqual(["validation.osdSystem"]);
    // Keep the board's current OSD: valid without a video system, nothing OSD-related is sent.
    await w!.get('[data-test="osd-hd-keep"]').trigger("click");
    await settle();
    expect(wizard.osd.mode).toBe("keep");
    expect(w!.get('[data-test="osd-kept"]').text()).toBe(en.osd.system.kept);
    expect(guards.osd.validate(wizard)).toEqual([]);
    expect(osdChoice(wizard.osd, false)).toBeNull();
    // Or pick analog.
    await w!.get('[data-osd-system="AUTO"]').setValue(true);
    await settle();
    expect(effectiveSystem(wizard.model, wizard.osd, wizard.fc)).toBe("AUTO");
    // Or update the firmware to add the HD OSD (confirmed first; see firmwareOptions.test.ts).
    await w!.get('[data-test="reflash-offer"]').trigger("click");
    await settle();
    expect(w!.get('[data-test="reflash-confirm"]').text()).toContain(en.reflash.confirmBody);
    await w!.get('[data-test="reflash-go"]').trigger("click");
    await settle();
    expect(nav.currentId).toBe("firmware");
    expect(wizard.ui.reflash).toEqual({ returnTo: "osd", requirements: { osd: { videoSystem: "HD" } } });
  });

  it("firmware without HD OSD: HD disabled with an explanation; a detected HD VTX is asked instead", async () => {
    await open(DJI, "osd-sd-only");
    expect(effectiveSystem(wizard.model, wizard.osd, wizard.fc)).toBeNull();
    expect(w!.get('[data-test="osd-hd-blocked"]').text()).toContain("built without the HD OSD");
    expect(w!.get('[data-osd-system="HD"]').attributes("disabled")).toBeDefined();
    expect(w!.get('[data-test="osd-hd-unsupported"]').text()).toContain("OSD (HD)");
    expect(guards.osd.validate(wizard).map((i) => i.key)).toEqual(["validation.osdSystem"]);
    // An HD choice kept from before (e.g. another board) blocks the step with the reason.
    wizard.osd.askedSystem = "HD";
    expect(guards.osd.validate(wizard).map((i) => i.key)).toEqual(["validation.osdHdUnsupported"]);
  });
});

describe("Review: config_plan refuses the HD OSD", () => {
  const RUST = "configuration error: HD OSD needs MSP DisplayPort, which this firmware build does not have";

  it("isHdOsdPlanError matches #67's CONFIG error only", () => {
    expect(isHdOsdPlanError({ code: "CONFIG", message: RUST })).toBe(true);
    expect(isHdOsdPlanError({ code: "CONFIG_UNSUPPORTED", message: RUST })).toBe(true);
    expect(isHdOsdPlanError({ code: "CONFIG", message: "no onboard SPI receiver (resource RX_SPI_CS not assigned)" })).toBe(false);
    expect(isHdOsdPlanError({ code: "SERIAL", message: RUST })).toBe(false);
  });

  async function toReview(): Promise<void> {
    await open(DJI, "osd-hd-plan-error");
    expect(w!.find('[data-test="osd-hd-unknown"]').exists()).toBe(true);
    await choose();
    expect(guards.osd.validate(wizard)).toEqual([]);
    nav.currentId = "review";
    await settle();
  }

  it("shows the error plainly with Back to OSD settings; back disables HD with the reason", async () => {
    await toReview();
    const panel = w!.get('[data-error-code="OSD_HD_UNSUPPORTED"]');
    expect(panel.text()).toContain(en.errors.OSD_HD_UNSUPPORTED.title);
    expect(panel.text()).toContain(en.errors.OSD_HD_UNSUPPORTED.body);
    const btn = panel.findAll("button").find((b) => b.text() === en.actions.backToOsd);
    expect(btn).toBeDefined();
    await btn!.trigger("click");
    await settle();
    expect(nav.currentId).toBe("osd");
    expect(wizard.osd.hdRejected).toBe(true);
    expect(effectiveSystem(wizard.model, wizard.osd, wizard.fc)).toBeNull();
    expect(w!.get('[data-test="osd-hd-blocked"]').text()).toContain(en.osd.system.hdRejected);
    expect(w!.get('[data-osd-system="HD"]').attributes("disabled")).toBeDefined();
    // Analog plans fine.
    await w!.get('[data-osd-system="AUTO"]').setValue(true);
    await settle();
    await choose();
    nav.currentId = "review";
    await settle();
    expect(w!.find('[data-error-code="OSD_HD_UNSUPPORTED"]').exists()).toBe(false);
  });

  it("detected by #78's details.reason, whatever the message says; the message match is only a fallback", async () => {
    const plan = vi.spyOn(api.config, "plan").mockRejectedValue({ code: "CONFIG", message: "osd: something else", details: { reason: "OSD_HD_NO_DISPLAYPORT" } });
    await toReview();
    expect(w!.find('[data-error-code="OSD_HD_UNSUPPORTED"]').exists()).toBe(true);
    // Another CONFIG error (no reason, other message) stays a CONFIG error.
    plan.mockRejectedValue({ code: "CONFIG", message: "no onboard SPI receiver" });
    nav.currentId = "osd";
    await settle();
    nav.currentId = "review";
    await settle();
    expect(w!.find('[data-error-code="OSD_HD_UNSUPPORTED"]').exists()).toBe(false);
    // Pre-#78 backend: no details, the message still matches.
    plan.mockRejectedValue({ code: "CONFIG", message: RUST });
    nav.currentId = "osd";
    await settle();
    nav.currentId = "review";
    await settle();
    expect(w!.find('[data-error-code="OSD_HD_UNSUPPORTED"]').exists()).toBe(true);
    plan.mockRestore();
  });

  it("in Hebrew", async () => {
    await toReview();
    setLocale("he");
    await settle();
    const panel = w!.get('[data-error-code="OSD_HD_UNSUPPORTED"]');
    expect(panel.text()).toContain(he.errors.OSD_HD_UNSUPPORTED.title);
    expect(panel.text()).toContain(he.errors.OSD_HD_UNSUPPORTED.step1);
    expect(panel.findAll("button").some((b) => b.text() === "חזרה להגדרות ה-OSD")).toBe(true);
  });

  it("a new identify asks the board again (e.g. after flashing HD OSD)", () => {
    const s = createWizardState();
    s.osd.hdRejected = true;
    setBoard(s, { ...(wizard.fc ?? {}), boardName: "X", uid: "1", buildOptions: [], hdOsdSupported: true } as never, [], null);
    expect(s.osd.hdRejected).toBe(false);
  });

  it("he/en: the new texts exist in both, and differ", () => {
    const keys = ["hdUnsupported", "hdBlocked", "hdRejected", "hdChoices", "keepCurrent", "kept", "hdUnknown"] as const;
    for (const k of keys) {
      expect(en.osd.system[k]).toBeTruthy();
      expect(he.osd.system[k]).toBeTruthy();
      expect(he.osd.system[k]).not.toBe(en.osd.system[k]);
    }
    for (const k of ["title", "body", "step1", "step2"] as const) expect(he.errors.OSD_HD_UNSUPPORTED[k]).toBeTruthy();
    expect(he.actions.backToOsd).toBeTruthy();
    expect(he.validation.osdHdUnsupported).toBeTruthy();
  });
});

describe("digital VTX + analog OSD", () => {
  it("is only a warning, not blocking", async () => {
    await open(DJI);
    expect(w!.find('[data-test="osd-analog-on-digital"]').exists()).toBe(false);
    await w!.get('[data-test="osd-change-system"]').trigger("click");
    await w!.get('[data-osd-system="AUTO"]').setValue(true);
    await settle();
    expect(analogOnDigital(wizard.model, "AUTO")).toBe(true);
    expect(w!.get('[data-test="osd-analog-on-digital"]').text()).toContain("the OSD is set to analog");
    await choose();
    expect(guards.osd.validate(wizard)).toEqual([]);
  });
});

describe("layout and defaults", () => {
  it("parseLayout tolerates extra keys and drops bad names, out-of-grid and duplicate elements", () => {
    const l = parseLayout(
      {
        videoSystem: "NTSC",
        cols: 30,
        rows: 13,
        extra: 1,
        elements: [
          { name: "MAIN_BATT_VOLTAGE", x: 1, y: 12, enabled: true, more: "x" },
          { name: "MAIN_BATT_VOLTAGE", x: 2, y: 2, enabled: false },
          { name: "RSSI_VALUE", x: 30, y: 1, enabled: true },
          { name: "ITEM_TIMER_2", x: 3, y: 13, enabled: true },
          { name: "vbat", x: 3, y: 3, enabled: true },
        ],
        future: true,
      },
      "NTSC",
    );
    expect(l).toEqual({ videoSystem: "NTSC", cols: 30, rows: 13, elements: [{ name: "MAIN_BATT_VOLTAGE", x: 1, y: 12, enabled: true }] });
  });

  it("grids per #66: PAL 30x16, NTSC / AUTO 30x13, HD 53x20", () => {
    const g = (v: "AUTO" | "PAL" | "NTSC" | "HD") => [mockOsdLayout(v).cols, mockOsdLayout(v).rows];
    expect([g("PAL"), g("NTSC"), g("AUTO"), g("HD")]).toEqual([[30, 16], [30, 13], [30, 13], [53, 20]]);
  });

  it("defaults come from the layout's `enabled` (no hard-coded set); GPS items only with GPS", () => {
    const l = mockOsdLayout("AUTO");
    const rec = recommendedEnabled(l, false);
    expect(rec.AVG_CELL_VOLTAGE).toBe(true);
    expect(rec.MAIN_BATT_VOLTAGE).toBe(false);
    expect("GPS_SATS" in rec).toBe(false);
    expect("ROLL_PIDS" in rec).toBe(false); // not offered by the UI
    expect(recommendedEnabled(l, true).GPS_SATS).toBe(true);
    const flipped = { ...l, elements: l.elements.map((e) => ({ ...e, enabled: !e.enabled })) };
    expect(recommendedEnabled(flipped, false).MAIN_BATT_VOLTAGE).toBe(true);
    expect(recommendedEnabled(flipped, false).AVG_CELL_VOLTAGE).toBe(false);
  });

  it("only elements in the layout and in the firmware's catalog are offered", () => {
    const l = { videoSystem: "PAL" as const, cols: 30, rows: 16, elements: [{ name: "MAIN_BATT_VOLTAGE", x: 1, y: 1, enabled: true }, { name: "ESC_TMP", x: 1, y: 2, enabled: true }] };
    expect(offeredElements(l, true).map((e) => e.name)).toEqual(["MAIN_BATT_VOLTAGE", "ESC_TMP"]);
    expect(offeredElements(l, true, ["MAIN_BATT_VOLTAGE", "LINK_QUALITY"]).map((e) => e.name)).toEqual(["MAIN_BATT_VOLTAGE"]);
    expect(offeredElements(null, true)).toEqual([]);
  });

  it("every UI element name is a Betaflight osd_items_e name without OSD_, and in every catalog", () => {
    for (const e of OSD_ELEMENTS) expect(e.name).toMatch(/^[A-Z0-9_]+$/);
    for (const fw of ["4.4.3", "4.5.2", "2025.12.1", "2026.6.2"]) for (const e of OSD_ELEMENTS) expect(mockFirmwareCatalog(fw).osdElements).toContain(e.name);
    expect(OSD_ELEMENTS.map((e) => e.name)).toEqual(expect.arrayContaining(["AVG_CELL_VOLTAGE", "MAIN_BATT_VOLTAGE", "ITEM_TIMER_2", "RSSI_DBM_VALUE"]));
  });
});

describe("preview", () => {
  it("analog: 4:3 canvas on the AUTO 30x13 grid, ticked elements only, with sample values", async () => {
    await open(ANALOG);
    await choose();
    const p = w!.get('[data-test="osd-preview"]');
    expect(p.classes()).toContain("osd-preview--analog");
    expect(p.attributes("dir")).toBe("ltr");
    expect([p.attributes("data-cols"), p.attributes("data-rows")]).toEqual(["30", "13"]);
    const on = Object.entries(wizard.osd.enabled).filter(([, v]) => v).map(([k]) => k);
    expect(previewNames().sort()).toEqual(on.sort());
    expect(w!.get('[data-preview="AVG_CELL_VOLTAGE"]').text()).toBe("4.12V");
    // Position from the layout: x=1, y=rows-2 → left 1/30, top 11/13.
    const style = w!.get('[data-preview="AVG_CELL_VOLTAGE"]').attributes("style");
    expect(style).toContain(`left: ${(1 / 30) * 100}%`);
    expect(style).toContain(`top: ${(11 / 13) * 100}%`);
    // Untick → gone; tick an optional one → shown.
    await w!.get('[data-osd="AVG_CELL_VOLTAGE"]').setValue(false);
    await w!.get('[data-osd="THROTTLE_POS"]').setValue(true);
    await settle();
    expect(previewNames()).not.toContain("AVG_CELL_VOLTAGE");
    expect(previewNames()).toContain("THROTTLE_POS");
  });

  it("HD: 16:9 canvas on the HD grid", async () => {
    await open(DJI);
    await choose();
    const p = w!.get('[data-test="osd-preview"]');
    expect(p.classes()).toContain("osd-preview--hd");
    expect([p.attributes("data-cols"), p.attributes("data-rows")]).toEqual(["53", "20"]);
  });

  it("any grid: PAL 30x16", async () => {
    await open(ANALOG);
    await w!.get('[data-test="osd-change-system"]').trigger("click");
    await w!.get('[data-osd-system="PAL"]').setValue(true);
    await settle();
    await choose();
    const p = w!.get('[data-test="osd-preview"]');
    expect([p.attributes("data-cols"), p.attributes("data-rows")]).toEqual(["30", "16"]);
    const style = w!.get('[data-preview="MAH_DRAWN"]').attributes("style");
    expect(style).toContain(`top: ${(14 / 16) * 100}%`);
  });

  it("arrow keys move an element, clamped to the grid, and mark it moved", async () => {
    await open(ANALOG);
    await choose();
    const el = w!.get('[data-preview="LINK_QUALITY"]'); // at (1,1)
    for (let i = 0; i < 4; i++) await el.trigger("keydown", { key: "ArrowLeft" });
    await el.trigger("keydown", { key: "ArrowUp" });
    await el.trigger("keydown", { key: "ArrowUp" });
    await settle();
    expect(wizard.osd.moved.LINK_QUALITY).toEqual({ x: 0, y: 0 });
    expect(w!.get('[data-test="osd-reset"]').attributes("disabled")).toBeUndefined();
    await w!.get('[data-test="osd-reset"]').trigger("click");
    await settle();
    expect(wizard.osd.moved).toEqual({});
  });

  it("Reset to recommended restores the layout's defaults", async () => {
    await open(ANALOG);
    await choose();
    const before = { ...wizard.osd.enabled };
    await w!.get('[data-osd="MAIN_BATT_VOLTAGE"]').setValue(true);
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
    s.layout = mockOsdLayout("AUTO");
    expect(osdChoice(s, false)).toBeNull();
    expect("osd" in withOsd(createWizardState().model, s)).toBe(false);
  });

  it("choose: #65 Osd with videoSystem; offered elements with enabled; x/y only for moved ones (clamped)", () => {
    const s = emptyOsdState();
    s.mode = "choose";
    s.layout = mockOsdLayout("NTSC");
    s.enabled = recommendedEnabled(s.layout, false);
    s.moved = { MAIN_BATT_VOLTAGE: { x: 99, y: 99 } };
    const c = osdChoice(s, false)!;
    expect(c.videoSystem).toBe("NTSC");
    expect(c.elements.map((e) => e.name)).not.toContain("ROLL_PIDS");
    expect(c.elements.map((e) => e.name)).not.toContain("GPS_SATS");
    expect(c.elements.find((e) => e.name === "MAIN_BATT_VOLTAGE")).toEqual({ name: "MAIN_BATT_VOLTAGE", enabled: false, x: 30 - 5, y: 12 });
    expect(c.elements.find((e) => e.name === "AVG_CELL_VOLTAGE")).toEqual({ name: "AVG_CELL_VOLTAGE", enabled: true });
    expect(withOsd(createWizardState().model, s).osd).toEqual(c);
  });

  it("the mock plan writes vcd_video_system / displayport and osd_<cli>_pos with BF's encoding", () => {
    expect(osdPosValue(1, 14, true)).toBe(1 | (14 << 5) | (0x7 << 11));
    expect(osdPosValue(40, 3, false)).toBe((40 & 31) | (1 << 10) | (3 << 5));
    const s = createWizardState();
    const osd = { videoSystem: "HD" as const, elements: [{ name: "MAIN_BATT_VOLTAGE", enabled: true, x: 40, y: 3 }, { name: "ITEM_TIMER_2", enabled: false }] };
    const cli = generateCli({ ...s.model, osd }, []);
    expect(cli.commands).toEqual(expect.arrayContaining(["feature OSD", "set vcd_video_system = HD", "set osd_displayport_device = MSP", "set osd_canvas_width = 53", "set osd_canvas_height = 20"]));
    expect(cli.commands).toContain(`set osd_vbat_pos = ${osdPosValue(40, 3, true)}`);
    expect(cli.commands).toContain(`set osd_tim_2_pos = ${osdPosValue(53 - 7, 1, false)}`);
    expect(cli.commands.some((c) => c.startsWith("set osd_") && c.includes("rssi"))).toBe(false);
    const pal = generateCli({ ...s.model, osd: { videoSystem: "PAL", elements: [] } }, []).commands;
    expect(pal).toEqual(expect.arrayContaining(["set vcd_video_system = PAL", "set osd_displayport_device = AUTO"]));
    expect(generateCli(s.model, []).commands.some((c) => c.startsWith("set osd_"))).toBe(false);
    expect(() => generateCli({ ...s.model, osd: { videoSystem: "PAL", elements: [{ name: "NOPE", enabled: true }] } }, [])).toThrow();
  });

  it("review shows the OSD row", async () => {
    await open(ANALOG);
    await choose();
    nav.currentId = "review";
    await settle();
    expect(w!.text()).toMatch(/OSD\s*\d+ items shown \(Analog, Auto \(recommended\)\)/);
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
