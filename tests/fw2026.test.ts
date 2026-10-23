// SPDX-License-Identifier: GPL-3.0-or-later
// Betaflight 2026.6 is a supported line since Config #83 (Dialect::V2026_6) and Device #82.
// `?mock=fw-2026`: the board runs 2026.6.2 (MSP API 1.48, UART1 = 51), the newest release.
// The mock catalog carries the real 2026.6 differences (config/osd.rs, modes.rs, catalog.rs).
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { resetDemoState } from "../src/api/fallback";
import { resetCatalog } from "../src/api/catalog";
import { i18n, setLocale } from "../src/i18n";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { installMock, uninstallMock } from "../src/mocks/install";
import { createScenarioMock, MOCK_CONFIG_LINES, mockFirmwareCatalog, type ScenarioMock } from "../src/mocks/scenarios";
import { lineSupport } from "../src/wizard/firmwareSupport";
import { nav, resetNav } from "../src/wizard/nav";
import { resetWizard, wizard } from "../src/wizard/state";
import { click, settle, walkToDone, WALK_TIMEOUT_MS } from "./walk";

/** The 12 OSD elements 2026.6 added over 2025.12 (82 -> 94), config/osd.rs. */
const NEW_2026_6 = [
  "POS_HOLD_READY", "BATTERY_PROFILE_NAME", "CUSTOM_SERIAL_TEXT", "WP_NUMBER", "WP_CURRENT_LAT", "WP_CURRENT_LON",
  "WP_CURRENT_ALT", "WP_DISTANCE", "WP_DIRECTION", "WP_NEXT_NUMBER", "WP_ETA", "NAV_MAP",
];

let w: VueWrapper | null = null;
let mock: ScenarioMock;

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
  document.body.innerHTML = "";
});

describe("mock catalog for Betaflight 2026.6 (Config #83)", () => {
  it("2026.6 is a supported line; 2026.12 (incl. alpha), 4.6 and 2027 are not, with the backend's wording", async () => {
    expect(MOCK_CONFIG_LINES).toContain("2026.6");
    const b = createScenarioMock({ latencyMs: 0, stepMs: 0 }).backend;
    const ask = (firmwareVersion: string) => b("config_firmware_catalog", { firmwareVersion });
    await expect(ask("2026.6.2")).resolves.toBeTruthy();
    await expect(ask("2026.6.0-RC1")).resolves.toBeTruthy();
    for (const v of ["2026.12.0-alpha", "2026.12.1-RC1", "2026.12.0", "2027.1.0", "4.6.0"]) {
      await expect(ask(v)).rejects.toMatchObject({ code: "UNSUPPORTED_FIRMWARE" });
    }
    await expect(ask("2026.12.0-alpha")).rejects.toMatchObject({
      message: "Betaflight 2026.12.0-alpha is not supported; supported lines are 4.4.x, 4.5.x, 2025.12.x and 2026.6.x",
    });
  });

  it("OSD: the 12 new 2026.6 elements on 2026.6 only; beepers and managed modes as on 2025.12", () => {
    const c26 = mockFirmwareCatalog("2026.6.2");
    const c25 = mockFirmwareCatalog("2025.12.1");
    const c45 = mockFirmwareCatalog("4.5.2");
    for (const n of NEW_2026_6) {
      expect(c26.osdElements).toContain(n);
      expect(c25.osdElements).not.toContain(n);
      expect(c45.osdElements).not.toContain(n);
    }
    expect(c26.osdElements.filter((n) => !NEW_2026_6.includes(n))).toEqual(c25.osdElements);
    expect(c26.beepers).toEqual(c25.beepers);
    expect(c26.beepers).not.toContain("RC_SMOOTHING_INIT_FAIL");
    expect(c26.modes).toEqual(c25.modes);
    // AUTOPILOT / WP CAPTURE exist on 2026.6 but are not wizard-managed.
    expect(c26.modes).not.toContain("AUTOPILOT");
    expect(c26.modes).not.toContain("WP CAPTURE");
    expect(c26.modes).toEqual(expect.arrayContaining(["ALTHOLD", "POSHOLD"]));
  });
});

async function toFirmware(): Promise<VueWrapper> {
  mock = installMock({ scenario: "fw-2026", latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await click(w, "next");
  await click(w, "connect");
  if (nav.currentId !== "firmware") await click(w, "next");
  expect(nav.currentId).toBe("firmware");
  return w;
}

describe("?mock=fw-2026 (board on Betaflight 2026.6.2)", () => {
  it(
    "up to date, nothing marked unsupported, and the keep path walks to Done with the 2026.6 dialect",
    async () => {
      await toFirmware();
      expect(wizard.fc?.firmwareVersion).toBe("2026.6.2");
      expect(wizard.fc?.apiVersion).toBe("1.48");
      expect(wizard.fc?.serialPorts.find((p) => p.name === "UART1")?.identifier).toBe(51);
      expect(w!.text()).toContain(en.firmware.upToDate);
      expect(w!.find('[data-test="fw-not-supported-yet"]').exists()).toBe(false);
      expect(w!.find('[data-test="fw-some-not-supported"]').exists()).toBe(false);
      expect((w!.get("#release").element as HTMLSelectElement).value).toBe("2026.6.2");
      expect(lineSupport(wizard, "2026.6.2")).toBe("supported");

      const seen = await walkToDone(w!, "keep");
      expect(seen).toContain("osd");
      expect(seen).toContain("apply");
      expect(nav.currentId).toBe("done");
      expect(mock.device.firmwareVersion).toBe("2026.6.2");
      expect(wizard.ui.script?.dialect).toBe("2026.6");
    },
    WALK_TIMEOUT_MS,
  );

  it(
    "in Hebrew",
    async () => {
      setLocale("he");
      await toFirmware();
      expect(w!.text()).toContain(he.firmware.upToDate);
      expect(w!.find('[data-test="fw-not-supported-yet"]').exists()).toBe(false);
      const seen = await walkToDone(w!, "keep");
      expect(seen).toContain("apply");
      expect(nav.currentId).toBe("done");
      expect(document.documentElement.dir).toBe("rtl");
    },
    WALK_TIMEOUT_MS,
  );
});
