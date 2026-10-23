// SPDX-License-Identifier: GPL-3.0-or-later
// Firmware build options (Device #74/#75): requirements from the answers, dropped options,
// BUILD_OPTIONS_UNAVAILABLE / REQUIRED_OPTION_UNSUPPORTED, and the mid-wizard
// "Update the firmware to add this" (OSD HD, receiver), with its safety limits.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { api, type Backend } from "../src/api/commands";
import { createEmitter } from "../src/api/events";
import { defaultMockHandlers } from "../src/api/mock";
import { installFallback, resetDemoState } from "../src/api/fallback";
import { resetCatalog } from "../src/api/catalog";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import { createScenarioMock, type Scenario, type ScenarioMock } from "../src/mocks/scenarios";
import { guards } from "../src/wizard/guards";
import { nav, resetNav } from "../src/wizard/nav";
import { effectiveSystem } from "../src/wizard/osd";
import { optionLabel, optionList } from "../src/wizard/optionNames";
import { finishReflash, missingOptions, reflashBlock, requestReflash, requirementsFor } from "../src/wizard/reflash";
import { createWizardState, resetWizard, setReceiverKind, setReceiverProtocol, wizard } from "../src/wizard/state";
import type { FcInfo } from "../src/types/model";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { click, settle, walkToDone, WALK_TIMEOUT_MS } from "./walk";

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

const t = (k: string) => String(i18n.global.t(k));
const te = (k: string) => i18n.global.te(k);

/** Boot the app on `scenario` and connect. */
async function boot(scenario: Scenario): Promise<VueWrapper> {
  mock = installMock({ scenario, latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await click(w, "next");
  await click(w, "connect");
  return w;
}
async function at(id: string): Promise<void> {
  nav.currentId = id;
  await settle();
}
function serialCrsf(): void {
  setReceiverKind(wizard, "SERIAL");
  setReceiverProtocol(wizard, "CRSF");
}
/** Walk from Connect with valid answers (serial CRSF receiver, DJI digital VTX) until `target`. */
async function walkTo(target: string): Promise<void> {
  await walkToDone(w!, "keep", {
    onStep: async (id) => {
      if (id === target) throw new Error("reached");
      if (id === "components" && wizard.model.receiver.kind === "NONE") {
        await w!.get('[data-rx-kind="SERIAL"]').setValue(true);
        await w!.get('[data-vtx-kind="DIGITAL"]').setValue(true);
        await settle();
      }
      if (id === "vtx" && !wizard.model.vtx.digitalSystem) {
        await w!.get('input[name="vtx-system"]').setValue(true);
        await settle();
      }
    },
  }).catch((e: Error) => {
    if (e.message !== "reached") throw e;
  });
  expect(nav.currentId).toBe(target);
}

describe("requirements from the answers", () => {
  it("receiver and OSD only when chosen; a mid-wizard request is added on top", () => {
    const s = createWizardState();
    expect(requirementsFor(s)).toBeNull();
    setReceiverKind(s, "SERIAL");
    setReceiverProtocol(s, "SBUS");
    expect(requirementsFor(s)).toEqual({ receiver: { kind: "SERIAL", protocol: "SBUS" }, osd: null });
    s.osd.mode = "choose";
    s.osd.askedSystem = "PAL";
    expect(requirementsFor(s)).toEqual({ receiver: { kind: "SERIAL", protocol: "SBUS" }, osd: { videoSystem: "PAL" } });
    s.osd.mode = "keep";
    expect(requirementsFor(s)?.osd).toBeNull();
    s.ui.reflash = { returnTo: "osd", requirements: { osd: { videoSystem: "HD" } } };
    expect(requirementsFor(s)).toEqual({ receiver: { kind: "SERIAL", protocol: "SBUS" }, osd: { videoSystem: "HD" } });
  });

  it("friendly option names where known, the raw name otherwise", () => {
    expect(optionLabel("USE_OSD_HD", t, te)).toBe(en.buildOptions.USE_OSD_HD);
    expect(optionLabel("USE_SOMETHING_NEW", t, te)).toBe("USE_SOMETHING_NEW");
    expect(optionLabel("not.a.key", t, te)).toBe("not.a.key");
    expect(optionList(["USE_WING", "MOCK_RX_CRSF"], t, te)).toBe(`${en.buildOptions.USE_WING}, MOCK_RX_CRSF`);
    setLocale("he");
    expect(optionLabel("USE_WING", t, te)).toBe(he.buildOptions.USE_WING);
  });

  it("every friendly name exists in en and he", () => {
    expect(Object.keys(he.buildOptions).sort()).toEqual(Object.keys(en.buildOptions).sort());
    expect(Object.keys(he.reflash).sort()).toEqual(Object.keys(en.reflash).sort());
  });
});

describe("Firmware step: build options", () => {
  it("passes the receiver/OSD answers to the update (flasher_required_options with the release)", async () => {
    await boot("happy");
    serialCrsf();
    await at("firmware");
    await click(w!, "update");
    expect(mock.device.requiredOptionsLog).toEqual(["4.5.2"]);
    expect(mock.device.addedOptions).toEqual(["MOCK_RX_CRSF"]);
    expect(wizard.ui.flashed).toBe(true);
    expect(w!.find('[data-test="fw-dropped"]').exists()).toBe(false);
  });

  it("nothing chosen yet: no requirements, the board keeps its own options", async () => {
    await boot("happy");
    await at("firmware");
    await click(w!, "update");
    expect(mock.device.requiredOptionsLog).toEqual([]);
    expect(wizard.ui.flashed).toBe(true);
  });

  it("a backend without flasher_required_options still updates (requirements skipped before anything is touched)", async () => {
    await boot("legacy-backend");
    serialCrsf();
    await at("firmware");
    await click(w!, "update");
    expect(wizard.ui.flashed).toBe(true);
    expect(w!.find("[data-error-code]").exists()).toBe(false);
  });

  it("dropped options are listed in plain words after the update", async () => {
    await boot("fw-dropped-options");
    await at("firmware");
    await click(w!, "update");
    const box = w!.get('[data-test="fw-dropped"]');
    expect(box.text()).toContain(en.firmware.droppedIntro);
    expect(box.text()).toContain(en.buildOptions.USE_WING);
    expect(box.text()).toContain(en.buildOptions.USE_LED_STRIP_64);
    expect(box.text()).toContain(en.firmware.droppedAfter);
    expect(wizard.fc?.buildOptions).not.toContain("USE_WING");
  });

  it("BUILD_OPTIONS_UNAVAILABLE: build server message with the release, nothing touched, Try again works", async () => {
    await boot("fw-options-unavailable");
    serialCrsf();
    await at("firmware");
    await click(w!, "update");
    const panel = w!.get("[data-error-code]");
    expect(panel.text()).toContain(en.errors.BUILD_OPTIONS_UNAVAILABLE.title);
    expect(panel.text()).toContain(en.errors.BUILD_OPTIONS_UNAVAILABLE.body.replace("{release}", "4.5.2"));
    expect(panel.text()).toContain(en.errors.BUILD_OPTIONS_UNAVAILABLE.step1);
    expect(mock.device.inDfu).toBe(false);
    expect(wizard.ui.sessionBackups).toHaveLength(0);
    expect(mock.device.firmwareVersion).not.toBe("4.5.2");
    await w!.get('[data-action="retry"]').trigger("click");
    await settle();
    expect(wizard.ui.flashed).toBe(true);
    expect(mock.device.firmwareVersion).toBe("4.5.2");
  });

  it("REQUIRED_OPTION_UNSUPPORTED: names what can't be included; 'Pick another version' goes to the picker", async () => {
    await boot("fw-option-unsupported");
    serialCrsf();
    await at("firmware");
    await click(w!, "update");
    const panel = w!.get("[data-error-code]");
    expect(panel.text()).toContain(en.errors.REQUIRED_OPTION_UNSUPPORTED.title);
    expect(panel.text()).toContain("Betaflight 4.5.2 can't be built with: MOCK_RX_CRSF.");
    expect(mock.device.inDfu).toBe(false);
    expect(wizard.ui.sessionBackups).toHaveLength(0);
    await w!.get('[data-action="pickRelease"]').trigger("click");
    await settle();
    expect(w!.find("[data-error-code]").exists()).toBe(false);
    expect(document.activeElement?.id).toBe("release");
    await w!.get('input[type="checkbox"]').setValue(true);
    await w!.get("#release").setValue("2025.12.0-RC1");
    await click(w!, "update");
    expect(wizard.ui.flashed).toBe(true);
    expect(mock.device.firmwareVersion).toBe("2025.12.0-RC1");
  });

  it("error copy uses friendly names and is translated", async () => {
    setLocale("he");
    await boot("fw-option-unsupported");
    setReceiverKind(wizard, "SERIAL");
    setReceiverProtocol(wizard, "CRSF");
    await at("firmware");
    await click(w!, "update");
    expect(w!.get("[data-error-code]").text()).toContain(he.errors.REQUIRED_OPTION_UNSUPPORTED.body.replace("{release}", "4.5.2").replace("{options}", "MOCK_RX_CRSF"));
    expect(w!.get('[data-action="pickRelease"]').text()).toBe(he.actions.pickRelease);
  });
});

describe("mid-wizard firmware update: OSD HD", () => {
  it("confirm, update with the HD OSD, come back to OSD with HD and every answer kept", async () => {
    await boot("osd-hd-reflash");
    await walkTo("osd");
    expect(wizard.fc?.hdOsdSupported).toBe(false);
    expect(effectiveSystem(wizard.model, wizard.osd, wizard.fc)).toBeNull();
    // Every answer except the board's own identity (new firmware version) stays.
    const answers = () => JSON.stringify({ ...wizard.model, fc: null });
    const before = answers();

    // Cancel first: nothing happens.
    await click(w!, "reflash-offer");
    const confirm = w!.get('[data-test="reflash-confirm"]');
    expect(confirm.text()).toContain(en.reflash.confirmTitle);
    expect(confirm.text()).toContain(en.reflash.confirmBody);
    expect(confirm.text()).toContain(en.reflash.confirmKeep);
    await click(w!, "reflash-cancel");
    expect(wizard.ui.reflash).toBeNull();
    expect(nav.currentId).toBe("osd");

    await click(w!, "reflash-offer");
    await click(w!, "reflash-go");
    expect(nav.currentId).toBe("firmware");
    expect(w!.get('[data-test="reflash-banner"]').text()).toContain(en.reflash.addsHd);
    expect(w!.find('[data-test="keep"]').exists()).toBe(false);

    await click(w!, "update");
    expect(mock.device.addedOptions).toContain("MOCK_OSD_HD");
    expect(mock.device.addedOptions).toContain("MOCK_RX_CRSF");
    expect(wizard.ui.sessionBackups).toHaveLength(1); // backup before the DFU reboot
    expect(nav.currentId).toBe("osd");
    expect(wizard.ui.reflash).toBeNull();
    expect(wizard.fc?.hdOsdSupported).toBe(true);
    expect(effectiveSystem(wizard.model, wizard.osd, wizard.fc)).toBe("HD");
    expect(w!.find('[data-test="osd-hd-blocked"]').exists()).toBe(false);
    expect(answers()).toBe(before);
    expect(wizard.model.fc?.firmwareVersion ?? wizard.fc?.firmwareVersion).toBe("4.5.2");
  });

  it("'Don't update, go back' on the Firmware step returns without flashing", async () => {
    await boot("osd-hd-reflash");
    await walkTo("osd");
    await click(w!, "reflash-offer");
    await click(w!, "reflash-go");
    await click(w!, "reflash-abort");
    expect(nav.currentId).toBe("osd");
    expect(wizard.ui.reflash).toBeNull();
    expect(mock.device.addedOptions).toEqual([]);
  });

  it("dropped options during a mid-wizard update: stay on Firmware with a 'Back to' button", async () => {
    await boot("fw-dropped-options");
    serialCrsf();
    await at("receiver");
    expect(requestReflash(wizard, { returnTo: "receiver", requirements: { receiver: { kind: "SERIAL", protocol: "CRSF" } } })).toBe(true);
    await at("firmware");
    await click(w!, "update");
    expect(nav.currentId).toBe("firmware");
    expect(w!.find('[data-test="fw-dropped"]').exists()).toBe(true);
    await click(w!, "reflash-return");
    expect(nav.currentId).toBe("receiver");
  });

  it("a mid-wizard update that fails keeps the request; Try again finishes it", async () => {
    await boot("fw-options-unavailable");
    await walkTo("osd");
    // fw-options-unavailable boards support HD; ask for it anyway via the API.
    requestReflash(wizard, { returnTo: "osd", requirements: { osd: { videoSystem: "HD" } } });
    await at("firmware");
    await click(w!, "update");
    expect(w!.get("[data-error-code]").text()).toContain(en.errors.BUILD_OPTIONS_UNAVAILABLE.title);
    expect(wizard.ui.reflash).not.toBeNull();
    await w!.get('[data-action="retry"]').trigger("click");
    await settle();
    expect(nav.currentId).toBe("osd");
  });
});

describe("mid-wizard firmware update: receiver", () => {
  it("receiver not in the build: warning + blocked; another protocol or an update fixes it", async () => {
    await boot("rx-option-missing");
    serialCrsf();
    await at("receiver");
    const box = w!.get('[data-test="rx-option-missing"]');
    expect(box.text()).toContain("doesn't include CRSF");
    expect(box.text()).toContain("MOCK_RX_CRSF");
    expect(guards.receiver.validate(wizard).map((i) => i.key)).toContain("validation.rxOptionMissing");

    await w!.get('[data-protocol="SBUS"]').setValue(true);
    await settle();
    expect(w!.find('[data-test="rx-option-missing"]').exists()).toBe(false);
    expect(guards.receiver.validate(wizard).map((i) => i.key)).not.toContain("validation.rxOptionMissing");

    await w!.get('[data-protocol="CRSF"]').setValue(true);
    await settle();
    await click(w!, "reflash-offer");
    await click(w!, "reflash-go");
    expect(w!.get('[data-test="reflash-banner"]').text()).toContain("CRSF");
    await click(w!, "update");
    expect(mock.device.addedOptions).toContain("MOCK_RX_CRSF");
    expect(nav.currentId).toBe("receiver");
    expect(wizard.model.receiver.protocol).toBe("CRSF");
    expect(w!.find('[data-test="rx-option-missing"]').exists()).toBe(false);
    expect(guards.receiver.validate(wizard).map((i) => i.key)).not.toContain("validation.rxOptionMissing");
  });

  it("unknown never blocks (backend can't say, or no build options)", async () => {
    await boot("legacy-backend");
    serialCrsf();
    await at("receiver");
    expect(w!.find('[data-test="rx-option-missing"]').exists()).toBe(false);
    expect(await missingOptions(wizard.fc, { receiver: { kind: "SERIAL", protocol: "CRSF" } })).toBeNull();
    expect(await missingOptions({ ...(wizard.fc as FcInfo), buildOptions: [] }, { receiver: { kind: "SERIAL", protocol: "CRSF" } })).toBeNull();
    expect(guards.receiver.validate(wizard).map((i) => i.key)).not.toContain("validation.rxOptionMissing");
  });

  it("a stale answer for another protocol is ignored", () => {
    const s = createWizardState();
    setReceiverKind(s, "SERIAL");
    setReceiverProtocol(s, "SBUS");
    s.ui.rxMissing = { kind: "SERIAL", protocol: "CRSF", missing: ["MOCK_RX_CRSF"] };
    expect(guards.receiver.validate(s).map((i) => i.key)).not.toContain("validation.rxOptionMissing");
  });
});

describe("mid-wizard firmware update: safety", () => {
  it(
    "not offered after config_apply in this session (walk to Done, then back to OSD)",
    async () => {
      await boot("reflash-after-apply");
      await walkToDone(w!, "keep", {
        onStep: async (id) => {
          if (id === "components" && wizard.model.receiver.kind === "NONE") {
            await w!.get('[data-rx-kind="SERIAL"]').setValue(true);
            await w!.get('[data-vtx-kind="DIGITAL"]').setValue(true);
            await settle();
          }
          if (id === "vtx" && !wizard.model.vtx.digitalSystem) {
            await w!.get('input[name="vtx-system"]').setValue(true);
            await settle();
          }
          if (id === "osd" && wizard.osd.mode === null) {
            // Before the apply the update is offered.
            expect(w!.find('[data-test="reflash-offer"]').exists()).toBe(true);
            await click(w!, "osd-hd-keep");
          }
        },
      });
      expect(nav.currentId).toBe("done");
      expect(wizard.ui.configApplied).toBe(true);
      await at("osd");
      expect(w!.get('[data-test="reflash-blocked"]').text()).toBe(en.reflash.blockedApplied);
      expect(w!.find('[data-test="reflash-offer"]').exists()).toBe(false);
      expect(reflashBlock(wizard)).toBe("applied");
      expect(requestReflash(wizard, { returnTo: "osd", requirements: { osd: { videoSystem: "HD" } } })).toBe(false);
      expect(wizard.ui.reflash).toBeNull();
    },
    WALK_TIMEOUT_MS,
  );

  it("never with simulated build options for a real board (mixed-backend guard)", async () => {
    const fc = (defaultMockHandlers.device_identify as () => FcInfo)();
    const real = async (cmd: string) => {
      if (cmd === "device_connect") return undefined;
      if (cmd === "device_identify") return fc;
      throw { code: "NOT_IMPLEMENTED", message: `${cmd} is not implemented yet` };
    };
    const m = createScenarioMock({ latencyMs: 0, stepMs: 0 });
    installFallback({ backend: m.backend, events: m.events.on }, { backend: real as unknown as Backend, events: createEmitter().on });
    await api.device.connect("COM5");
    wizard.fc = await api.device.identify();
    expect(reflashBlock(wizard)).toBeNull();
    // The receiver check gets a simulated answer: unknown, never blocks...
    expect(await missingOptions({ ...wizard.fc, buildOptions: ["MOCK_RX_SBUS"] }, { receiver: { kind: "SERIAL", protocol: "CRSF" } })).toBeNull();
    // ...and the update is no longer offered: it would flash with simulated options.
    expect(reflashBlock(wizard)).toBe("mixed");
    expect(requestReflash(wizard, { returnTo: "osd", requirements: { osd: { videoSystem: "HD" } } })).toBe(false);
  });

  it("finishReflash after Cancel does not change the OSD answer", () => {
    const s = createWizardState();
    s.ui.reflash = { returnTo: "osd", requirements: { osd: { videoSystem: "HD" } } };
    s.osd.askedSystem = "PAL";
    expect(finishReflash(s, false)).toBe("osd");
    expect(s.osd.askedSystem).toBe("PAL");
    expect(s.ui.reflash).toBeNull();
  });
});

describe("full walk with a mid-wizard update", () => {
  it(
    "HD OSD added on the OSD step, the walk continues to Done with the HD OSD applied",
    async () => {
      await boot("osd-hd-reflash");
      let reflashed = false;
      await walkToDone(w!, "keep", {
        onStep: async (id) => {
          if (id === "components" && wizard.model.receiver.kind === "NONE") {
            await w!.get('[data-rx-kind="SERIAL"]').setValue(true);
            await w!.get('[data-vtx-kind="DIGITAL"]').setValue(true);
            await w!.get('[data-test="gps"]').setValue(true);
            await settle();
          }
          if (id === "vtx" && !wizard.model.vtx.digitalSystem) {
            await w!.get('input[name="vtx-system"]').setValue(true);
            await settle();
          }
          if (id === "osd" && !reflashed) {
            reflashed = true;
            await click(w!, "reflash-offer");
            await click(w!, "reflash-go");
            expect(nav.currentId).toBe("firmware");
            await click(w!, "update");
            expect(nav.currentId).toBe("osd");
          }
        },
      });
      expect(nav.currentId).toBe("done");
      expect(reflashed).toBe(true);
      expect(mock.device.addedOptions).toContain("MOCK_OSD_HD");
      expect(wizard.ui.script?.commands).toContain("set osd_displayport_device = MSP");
      expect(wizard.ui.configApplied).toBe(true);
    },
    WALK_TIMEOUT_MS,
  );
});
