// SPDX-License-Identifier: GPL-3.0-or-later
// READ_PROTECTED: nothing is erased without an explicit typed confirmation; then flashing continues.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { api } from "../src/api/commands";
import { flasherNext } from "../src/api/device";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import type { ScenarioMock } from "../src/mocks/scenarios";
import { removeReadProtection, runFirmwareUpdate } from "../src/wizard/flash";
import { motorTest } from "../src/wizard/motorTest";
import { nav, resetNav } from "../src/wizard/nav";
import { resetWizard, wizard } from "../src/wizard/state";
import { click, settle, walkToDone } from "./walk";

let mock: ScenarioMock;
let w: VueWrapper | null = null;
const noop = { onStage: () => {}, onProgress: () => {} };
const poll = { tries: 3, intervalMs: 0 };

beforeEach(() => {
  resetWizard();
  resetNav();
  setLocale("en");
  mock = installMock({ scenario: "read-protected", latencyMs: 0, stepMs: 0 });
});
afterEach(async () => {
  await motorTest.stop("unmount");
  w?.unmount();
  w = null;
  uninstallMock();
});

describe("read protection (orchestration)", () => {
  it("flash stops with READ_PROTECTED before erasing; removal + retry flashes", async () => {
    const device = await api.device.identify();
    let serial: string | null = null;
    await expect(runFirmwareUpdate(device, "4.5.2", { ...noop, onDfu: (d) => (serial = d.serial) }, poll)).rejects.toMatchObject({ code: "READ_PROTECTED" });
    expect(serial).toBe("MOCKDFU1");
    expect(mock.device.unprotectLog).toEqual([]); // never automatic
    expect(mock.device.firmwareVersion).toBe("4.5.1");

    await removeReadProtection(serial, poll);
    expect(mock.device.unprotectLog).toEqual([true]);
    const r = await runFirmwareUpdate(device, "4.5.2", noop, poll);
    expect(r.fc.firmwareVersion).toBe("4.5.2");
  });

  it("the backend refuses without confirmErase (and the mock records it)", async () => {
    await expect(flasherNext.removeReadProtection("MOCKDFU1", false)).rejects.toMatchObject({ code: "ERASE_CONFIRMATION_REQUIRED" });
    expect(mock.device.readProtected).toBe(true);
  });
});

async function toFirmwareUpdate(): Promise<void> {
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await click(w, "next");
  await click(w, "connect");
  await click(w, "next");
  expect(nav.currentId).toBe("firmware");
  await click(w, "update");
}

describe("read-protected screen", () => {
  it("requires the checkbox AND the typed word, then erases and continues into flashing and on to Done", async () => {
    await toFirmwareUpdate();
    const panel = w!.get('[data-test="read-protected"]');
    expect(panel.text()).toContain("ERASES THE WHOLE CHIP");
    expect(panel.text()).toContain("A backup of your settings was saved");
    const erase = () => w!.get('[data-test="rp-erase"]').element as HTMLButtonElement;
    expect(erase().disabled).toBe(true);

    await w!.get('[data-test="rp-confirm"]').setValue("erase");
    expect(erase().disabled).toBe(true); // typed only
    await w!.get('[data-test="rp-confirm"]').setValue("ERAS");
    await w!.get('[data-test="rp-understood"]').setValue(true);
    expect(erase().disabled).toBe(true); // wrong word
    expect(mock.device.unprotectLog).toEqual([]);

    await w!.get('[data-test="rp-confirm"]').setValue(" erase ");
    expect(erase().disabled).toBe(false);
    await click(w!, "rp-erase");
    expect(mock.device.unprotectLog).toEqual([true]);
    expect(w!.find('[data-test="read-protected"]').exists()).toBe(false);
    expect(wizard.ui.flashed).toBe(true);
    expect(wizard.fc?.firmwareVersion).not.toBe("4.5.1");
    expect(wizard.ui.sessionBackups).toHaveLength(1);

    await walkToDone(w!, "keep");
    expect(nav.currentId).toBe("done");
  });

  it("Hebrew asks for the Hebrew word", async () => {
    setLocale("he");
    await toFirmwareUpdate();
    expect(w!.get('[data-test="read-protected"]').text()).toContain("מוחקת את כל השבב");
    await w!.get('[data-test="rp-understood"]').setValue(true);
    await w!.get('[data-test="rp-confirm"]').setValue("ERASE");
    expect((w!.get('[data-test="rp-erase"]').element as HTMLButtonElement).disabled).toBe(true);
    await w!.get('[data-test="rp-confirm"]').setValue("מחיקה");
    expect((w!.get('[data-test="rp-erase"]').element as HTMLButtonElement).disabled).toBe(false);
  });
});
