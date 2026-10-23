// SPDX-License-Identifier: GPL-3.0-or-later
// WinUSB install that replaces another DFU driver (DriverStatus.devices[].replacesDriver,
// #44): warn, name the driver, and install only after an explicit confirmation.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import type { ScenarioMock } from "../src/mocks/scenarios";
import DriverReplaceConfirm from "../src/components/DriverReplaceConfirm.vue";
import { installDfuDriverSafely } from "../src/wizard/dfuDriver";
import { motorTest } from "../src/wizard/motorTest";
import { nav, resetNav } from "../src/wizard/nav";
import { resetWizard, wizard } from "../src/wizard/state";
import type { UsbDriverInfo } from "../src/types/model";
import { click, settle } from "./walk";

let mock: ScenarioMock;
let w: VueWrapper | null = null;

beforeEach(() => {
  resetWizard();
  resetNav();
  setLocale("en");
});
afterEach(async () => {
  await motorTest.stop("unmount");
  w?.unmount();
  w = null;
  uninstallMock();
});

describe("installDfuDriverSafely", () => {
  it("does not install without confirmation when another driver would be replaced", async () => {
    mock = installMock({ scenario: "driver-replace", latencyMs: 0, stepMs: 0 });
    mock.device.inDfu = true;
    const r = await installDfuDriverSafely(false);
    expect(r.installed).toBe(false);
    expect(r.installed ? [] : r.replaces.map((d) => [d.service, d.replacesDriver])).toEqual([["STTub30", true]]);
    expect(mock.device.driverInstalls).toBe(0);
    expect(mock.device.dfuOtherDriver).toBe("STTub30");
  });

  it("installs once confirmed", async () => {
    mock = installMock({ scenario: "driver-replace", latencyMs: 0, stepMs: 0 });
    mock.device.inDfu = true;
    expect(await installDfuDriverSafely(true)).toEqual({ installed: true });
    expect(mock.device.driverInstalls).toBe(1);
    expect(mock.device.dfuDriverOk).toBe(true);
  });

  it("no other driver bound: installs without asking", async () => {
    mock = installMock({ scenario: "driver-missing", latencyMs: 0, stepMs: 0 });
    mock.device.inDfu = true;
    expect(await installDfuDriverSafely(false)).toEqual({ installed: true });
    expect(mock.device.driverInstalls).toBe(1);
  });
});

const dev = (over: Partial<UsbDriverInfo> = {}): UsbDriverInfo => ({
  vid: 0x0483, pid: 0xdf11, name: "STM32 BOOTLOADER", kind: "DFU", instanceId: null, service: "STTub30",
  driverOk: false, action: "INSTALL_DFU_DRIVER", guidanceIndex: null, replacesDriver: true, ...over,
});

describe("DriverReplaceConfirm", () => {
  for (const locale of ["en", "he"] as const) {
    it(`names the replaced driver and needs the checkbox before confirm (${locale})`, async () => {
      setLocale(locale);
      const c = mount(DriverReplaceConfirm, { props: { replaces: [dev()] }, global: { plugins: [i18n] } });
      const text = c.text();
      expect(text).toContain("STTub30");
      expect(text).toContain("STM32CubeProgrammer");
      expect(text).toContain("STM32 BOOTLOADER");
      expect(text).toContain(locale === "he" ? "עלולות להפסיק לזהות את הלוח" : "may stop seeing the board");
      const btn = () => c.get('[data-test="drv-install"]').element as HTMLButtonElement;
      expect(btn().disabled).toBe(true);
      await c.get('[data-test="drv-install"]').trigger("click");
      expect(c.emitted("confirm")).toBeUndefined();
      await c.get('[data-test="drv-understood"]').setValue(true);
      expect(btn().disabled).toBe(false);
      await c.get('[data-test="drv-install"]').trigger("click");
      expect(c.emitted("confirm")).toHaveLength(1);
      c.unmount();
    });
  }

  it("an unknown driver name still gets a clear sentence", () => {
    const c = mount(DriverReplaceConfirm, { props: { replaces: [dev({ service: null })] }, global: { plugins: [i18n] } });
    expect(c.text()).toContain("currently uses the current driver");
    c.unmount();
  });
});

describe("firmware update with STTub30 bound", () => {
  it("DRIVER_MISSING -> Install asks first; Cancel installs nothing; confirm installs and flashing continues", async () => {
    mock = installMock({ scenario: "driver-replace", latencyMs: 0, stepMs: 0 });
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    await click(w, "next");
    await click(w, "connect");
    await click(w, "next");
    expect(nav.currentId).toBe("firmware");
    await click(w, "update");
    expect(w.find('[data-error-code="DRIVER_MISSING"]').exists()).toBe(true);

    const installBtn = () => w!.findAll('[data-error-code="DRIVER_MISSING"] button').find((b) => b.text() === i18n.global.t("actions.installDriver"))!;
    await installBtn().trigger("click");
    await settle();
    expect(w.get('[data-test="driver-replace"]').text()).toContain("STTub30");
    expect(mock.device.driverInstalls).toBe(0);

    await click(w, "drv-cancel");
    expect(w.find('[data-test="driver-replace"]').exists()).toBe(false);
    expect(mock.device.driverInstalls).toBe(0);

    await installBtn().trigger("click");
    await settle();
    await click(w, "drv-install"); // disabled: not confirmed
    expect(mock.device.driverInstalls).toBe(0);
    await w.get('[data-test="drv-understood"]').setValue(true);
    await click(w, "drv-install");
    expect(mock.device.driverInstalls).toBe(1);
    expect(wizard.ui.flashed).toBe(true);
  });
});
