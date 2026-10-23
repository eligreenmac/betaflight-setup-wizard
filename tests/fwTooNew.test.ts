// SPDX-License-Identifier: GPL-3.0-or-later
// Firmware lines come from the config engine (config_firmware_catalog answers
// UNSUPPORTED_FIRMWARE for a line it can't set up), not from a list in the UI.
// `fw-too-new`: the board runs Betaflight 2026.12.0-alpha, 2026.12.x releases are offered by
// the build server but the backend (4.4 / 4.5 / 2025.12 / 2026.6 since Config #83) can't set
// them up yet. Same situation v0.1.0-2 hit with 2026.6.2.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { resetDemoState } from "../src/api/fallback";
import { resetCatalog } from "../src/api/catalog";
import { i18n, setLocale } from "../src/i18n";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { installMock, uninstallMock } from "../src/mocks/install";
import { MOCK_CONFIG_LINES, type ScenarioMock } from "../src/mocks/scenarios";
import { checkFirmwareLines, checkFirmwareSupport, firmwareLine, lineSupport } from "../src/wizard/firmwareSupport";
import { firmwareOk, guards } from "../src/wizard/guards";
import { nav, resetNav } from "../src/wizard/nav";
import { createWizardState, resetWizard, setBoard, wizard } from "../src/wizard/state";
import type { FcInfo } from "../src/types/model";
import { click, settle, walkToDone, WALK_TIMEOUT_MS } from "./walk";

let w: VueWrapper | null = null;
let mock: ScenarioMock;
const savedLines = [...MOCK_CONFIG_LINES];

beforeEach(() => {
  resetDemoState();
  resetWizard();
  resetNav();
  resetCatalog();
  setLocale("en");
});
afterEach(() => {
  w?.unmount();
  w = null;
  uninstallMock();
  resetDemoState();
  MOCK_CONFIG_LINES.splice(0, MOCK_CONFIG_LINES.length, ...savedLines);
  document.body.innerHTML = "";
});

async function toFirmware(): Promise<VueWrapper> {
  mock = installMock({ scenario: "fw-too-new", latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await click(w, "next");
  await click(w, "connect");
  if (nav.currentId !== "firmware") await click(w, "next");
  expect(nav.currentId).toBe("firmware");
  return w;
}

const fc = (firmwareVersion: string): FcInfo =>
  ({ fcVariant: "BTFL", firmwareVersion, apiVersion: "1.46", boardName: "SPEEDYBEEF405V4", manufacturerId: null, mcu: null, uid: null, buildOptions: [], hdOsdSupported: null, serialPorts: [], rxSpi: { present: null, expresslrsPins: false, protocol: null, featureEnabled: null, resources: [] } }) as FcInfo;

describe("firmwareLine / checkFirmwareSupport", () => {
  it("line of a version", () => {
    expect(firmwareLine("2026.6.2")).toBe("2026.6");
    expect(firmwareLine("4.5.1")).toBe("4.5");
    expect(firmwareLine("2025.12.0-RC1")).toBe("2025.12");
    expect(firmwareLine("garbage")).toBe("");
    expect(firmwareLine(null)).toBe("");
  });

  it("asks once per line; only UNSUPPORTED_FIRMWARE says no; other failures stay unknown and are asked again", async () => {
    const s = createWizardState();
    const asked: string[] = [];
    const ask = async (v: string) => {
      asked.push(v);
      if (v.startsWith("2026.6")) throw { code: "UNSUPPORTED_FIRMWARE", message: `Betaflight ${v} is not supported` };
      if (v.startsWith("2027")) throw { code: "NOT_IMPLEMENTED", message: "old backend" };
      return {};
    };
    await checkFirmwareLines(s, ["2026.6.2", "2026.6.1", "4.5.2", "4.5.1", "2027.1.0"], ask);
    expect(asked).toEqual(["2026.6.2", "4.5.2", "2027.1.0"]);
    expect(lineSupport(s, "2026.6.0")).toBe("unsupported");
    expect(lineSupport(s, "4.5.0")).toBe("supported");
    expect(lineSupport(s, "2027.1.0")).toBe("unknown");
    expect(await checkFirmwareSupport(s, "2027.1.3", ask)).toBe("unknown");
    expect(asked).toHaveLength(4);
  });

  it("the guard follows the backend's answer: blocked when unsupported, open as soon as the backend supports the line", async () => {
    const s = createWizardState();
    setBoard(s, fc("2026.6.2"), [], "COM5");
    s.ui.firmwareDecision = "keep";
    expect(firmwareOk(s)).toBe(true); // not asked yet = unknown = not blocked
    await checkFirmwareSupport(s, "2026.6.2", async () => Promise.reject({ code: "UNSUPPORTED_FIRMWARE", message: "x" }));
    expect(firmwareOk(s)).toBe(false);
    expect(guards.firmware.validate(s)).toEqual([{ key: "validation.firmwareNotSupportedYet", params: { version: "2026.6.2" } }]);
    // A backend that has added 2026.6: same UI code, no list to edit.
    const s2 = createWizardState();
    setBoard(s2, fc("2026.6.2"), [], "COM5");
    s2.ui.firmwareDecision = "keep";
    await checkFirmwareSupport(s2, "2026.6.2", async () => ({}));
    expect(guards.firmware.validate(s2)).toEqual([]);
  });
});

describe("?mock=fw-too-new (board on Betaflight 2026.12.0-alpha)", () => {
  it(
    "says plainly the wizard can't set it up yet, only offers versions it can, and the update walks to Done",
    async () => {
      await toFirmware();
      expect(wizard.fc?.firmwareVersion).toBe("2026.12.0-alpha");
      const callout = w!.get('[data-test="fw-not-supported-yet"]');
      expect(callout.text()).toContain(en.firmware.notSupportedYetTitle);
      expect(callout.text()).toContain(en.firmware.notSupportedYetBody.replace("{version}", "2026.12.0-alpha"));
      expect(w!.text()).not.toContain(en.firmware.unsupportedTitle); // not "too old"
      expect(w!.text()).not.toContain(en.firmware.upToDate);
      expect(w!.find('[data-test="keep"]').exists()).toBe(false);

      // The picker: 2026.12.x shown but disabled and marked; the default is the newest stable the wizard can set up.
      const opts = w!.findAll("#release option").map((o) => ({ v: (o.element as HTMLOptionElement).value, d: (o.element as HTMLOptionElement).disabled, text: o.text() }));
      expect(opts.filter((o) => o.d).map((o) => o.v)).toEqual(["2026.12.0"]);
      expect(opts.find((o) => o.v === "2026.12.0")!.text).toContain(en.firmware.lineNotSupported);
      expect(opts.find((o) => o.v === "4.5.2")!.text).not.toContain(en.firmware.lineNotSupported);
      expect(opts.find((o) => o.v === "2026.6.2")!.text).not.toContain(en.firmware.lineNotSupported); // supported since Config #83
      expect(opts.some((o) => o.v === "2026.12.0-alpha")).toBe(false); // pre-release, hidden by default
      expect((w!.get("#release").element as HTMLSelectElement).value).toBe("2026.6.2");
      expect(wizard.ui.latestRelease).toBe("2026.6.2");
      expect(w!.get('[data-test="fw-some-not-supported"]').text()).toBe(en.firmware.someNotSupported);
      expect(w!.get('[data-test="update"]').text()).toBe(en.firmware.installVersion.replace("{version}", "2026.6.2"));

      // Next is blocked with the plain reason.
      await click(w!, "next");
      expect(nav.currentId).toBe("firmware");
      expect(w!.text()).toContain(en.validation.firmwareNotSupportedYet.replace("{version}", "2026.12.0-alpha"));

      // Install the offered version and go all the way.
      const seen = await walkToDone(w!, "update");
      expect(mock.device.firmwareVersion).toBe("2026.6.2");
      expect(lineSupport(wizard, "2026.6.2")).toBe("supported");
      expect(seen).toContain("apply");
      expect(nav.currentId).toBe("done");
    },
    WALK_TIMEOUT_MS,
  );

  it("in Hebrew", async () => {
    setLocale("he");
    await toFirmware();
    expect(w!.get('[data-test="fw-not-supported-yet"]').text()).toContain(he.firmware.notSupportedYetBody.replace("{version}", "2026.12.0-alpha"));
    expect(w!.get('[data-test="update"]').text()).toBe(he.firmware.installVersion.replace("{version}", "2026.6.2"));
    expect(w!.get("#release option[value='2026.12.0']").text()).toContain(he.firmware.lineNotSupported);
    await click(w!, "next");
    expect(w!.text()).toContain(he.validation.firmwareNotSupportedYet.replace("{version}", "2026.12.0-alpha"));
  });

  it("data-driven: once the (mock) config engine supports 2026.12, the same board just works", async () => {
    MOCK_CONFIG_LINES.push("2026.12");
    await toFirmware();
    expect(w!.find('[data-test="fw-not-supported-yet"]').exists()).toBe(false);
    expect(w!.find('[data-test="fw-some-not-supported"]').exists()).toBe(false);
    expect((w!.get("#release").element as HTMLSelectElement).value).toBe("2026.12.0");
    expect(w!.text()).toContain(en.firmware.upToDate);
    await click(w!, "keep");
    await click(w!, "next");
    expect(nav.currentId).not.toBe("firmware");
  });
});
