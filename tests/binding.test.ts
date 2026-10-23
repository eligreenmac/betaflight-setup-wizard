// SPDX-License-Identifier: GPL-3.0-or-later
// ExpressLRS binding step: phrase -> UID via api.config.elrsUid, never stored or logged.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../src/App.vue";
import type { Backend } from "../src/api/commands";
import type { DeviceBackend } from "../src/api/device";
import { createEmitter } from "../src/api/events";
import { demoState, installFallback, resetDemoState } from "../src/api/fallback";
import { i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import { createScenarioMock, type Scenario } from "../src/mocks/scenarios";
import { visibleSteps } from "../src/wizard/engine";
import { guardFor } from "../src/wizard/guards";
import { motorTest } from "../src/wizard/motorTest";
import { nav, resetNav } from "../src/wizard/nav";
import { createWizardState, resetWizard, setReceiverKind, setReceiverProtocol, wizard } from "../src/wizard/state";
import { click, settle, walkToDone } from "./walk";

const PHRASE = "my very secret phrase";
let w: VueWrapper | null = null;

beforeEach(() => {
  resetWizard();
  resetNav();
  resetDemoState();
  setLocale("en");
});
afterEach(async () => {
  await motorTest.stop("unmount");
  w?.unmount();
  w = null;
  uninstallMock();
  resetDemoState();
  vi.restoreAllMocks();
});

describe("binding step rules", () => {
  it("is shown for onboard ExpressLRS and UART CRSF only", () => {
    const s = createWizardState();
    const ids = () => visibleSteps(s.model).map((x) => x.id);
    setReceiverKind(s, "SERIAL");
    expect(ids()).toContain("binding"); // CRSF default
    setReceiverProtocol(s, "SBUS");
    expect(ids()).not.toContain("binding");
    setReceiverKind(s, "SPI");
    expect(ids()).toContain("binding");
  });

  it("asks ELRS vs Crossfire on a UART, then how to bind; onboard needs the phrase hashed", () => {
    const s = createWizardState();
    const keys = () => guardFor("binding").validate(s).map((i) => i.key);
    setReceiverKind(s, "SERIAL");
    expect(keys()).toEqual(["validation.crsfFamilyRequired"]);
    s.ui.crsfFamily = "CROSSFIRE";
    expect(keys()).toEqual([]);
    s.ui.crsfFamily = "ELRS";
    expect(keys()).toEqual(["validation.bindingRequired"]);
    s.ui.elrsBinding = "phrase";
    expect(keys()).toEqual([]); // the UART receiver keeps its own phrase

    setReceiverKind(s, "SPI");
    expect(s.ui.elrsBinding).toBeNull(); // reset with the receiver
    s.ui.elrsBinding = "phrase";
    expect(keys()).toEqual(["validation.phraseNotSet"]);
    s.model.receiver.elrsUid = [1, 2, 3, 4, 5, 6];
    expect(keys()).toEqual([]);
    setReceiverProtocol(s, "EXPRESSLRS"); // same protocol: kept
    expect(s.model.receiver.elrsUid).toEqual([1, 2, 3, 4, 5, 6]);
    setReceiverKind(s, "SERIAL");
    expect(s.model.receiver.elrsUid).toBeNull();
  });
});

async function boot(scenario: Scenario): Promise<void> {
  installMock({ scenario, latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
}
async function toBinding(kind: "SPI" | "SERIAL"): Promise<void> {
  await walkUntil("binding", async (id) => {
    if (id === "components" && wizard.model.receiver.kind === "NONE") {
      await w!.get(`[data-rx-kind="${kind}"]`).setValue(true);
      await settle();
    }
  });
}
/** walkToDone, stopping when `target` is reached. */
async function walkUntil(target: string, onStep: (id: string) => Promise<void>): Promise<void> {
  await walkToDone(w!, "keep", {
    onStep: async (id) => {
      if (id === target) throw new Error("reached");
      await onStep(id);
    },
  }).catch((e: Error) => {
    if (e.message !== "reached") throw e;
  });
  expect(nav.currentId).toBe(target);
}

describe("binding screen", () => {
  it("onboard ExpressLRS: hashes the phrase, keeps only the UID, never logs or stores the phrase, reaches Done", async () => {
    const spies = (["log", "info", "warn", "error", "debug"] as const).map((m) => vi.spyOn(console, m));
    await boot("onboard-elrs");
    await toBinding("SPI");
    expect(w!.text()).toContain("must be exactly the same as the phrase on your transmitter");
    expect(w!.find('[data-family="ELRS"]').exists()).toBe(false);

    await w!.get('[data-binding="phrase"]').setValue(true);
    const input = w!.get('[data-test="phrase"]');
    expect(input.attributes("type")).toBe("password");
    expect(input.attributes("autocomplete")).toBe("off");
    await click(w!, "next");
    expect(nav.currentId).toBe("binding"); // phrase not set yet

    await input.setValue(PHRASE);
    await click(w!, "phrase-apply");
    expect(wizard.model.receiver.elrsUid).toHaveLength(6);
    expect(w!.find('[data-test="phrase-set"]').exists()).toBe(true);
    expect(w!.find('[data-test="phrase"]').exists()).toBe(false);

    await walkToDone(w!, "keep");
    expect(nav.currentId).toBe("done");
    expect(wizard.ui.script?.commands.some((c) => c.startsWith("set expresslrs_uid = "))).toBe(true);

    expect(JSON.stringify(wizard)).not.toContain(PHRASE);
    expect(JSON.stringify({ ...localStorage })).not.toContain(PHRASE);
    expect(JSON.stringify({ ...sessionStorage })).not.toContain(PHRASE);
    expect(document.body.innerHTML).not.toContain(PHRASE);
    for (const s of spies) expect(JSON.stringify(s.mock.calls)).not.toContain(PHRASE);
  });

  it("the phrase input is cleared even when hashing fails", async () => {
    await boot("onboard-elrs");
    await toBinding("SPI");
    await w!.get('[data-binding="phrase"]').setValue(true);
    const { api } = await import("../src/api/commands");
    vi.spyOn(api.config, "elrsUid").mockRejectedValueOnce({ code: "INTERNAL", message: "boom" });
    await w!.get('[data-test="phrase"]').setValue(PHRASE);
    await click(w!, "phrase-apply");
    expect((w!.get('[data-test="phrase"]').element as HTMLInputElement).value).toBe("");
    expect(wizard.model.receiver.elrsUid ?? null).toBeNull();
    expect(w!.find('[role="alert"]').exists()).toBe(true);
  });

  it("skip the phrase: bind with the button keeps the board's UID", async () => {
    await boot("onboard-elrs");
    await toBinding("SPI");
    await w!.get('[data-binding="button"]').setValue(true);
    expect(w!.get('[data-test="bind-steps"]').text()).toContain("not the BOOT button");
    expect(wizard.model.receiver.elrsUid ?? null).toBeNull();
    await click(w!, "next");
    expect(nav.currentId).not.toBe("binding");
  });

  it("UART receiver: Crossfire needs nothing; ExpressLRS explains where the phrase is set (no input)", async () => {
    await boot("happy");
    await toBinding("SERIAL");
    await w!.get('[data-family="CROSSFIRE"]').setValue(true);
    expect(w!.find('[data-test="crossfire-note"]').exists()).toBe(true);
    expect(w!.find('[data-binding="phrase"]').exists()).toBe(false);
    await w!.get('[data-family="ELRS"]').setValue(true);
    await w!.get('[data-binding="phrase"]').setValue(true);
    expect(w!.find('[data-test="phrase"]').exists()).toBe(false);
    expect(w!.get('[data-test="uart-phrase"]').text()).toContain("ExpressLRS Configurator");
    await w!.get('[data-binding="button"]').setValue(true);
    expect(w!.get('[data-test="bind-steps"]').text()).toContain("three times");
  });

  it("demo mode: config_elrs_uid falls back to the mock when the backend lacks it", async () => {
    const mock = createScenarioMock({ scenario: "onboard-elrs", latencyMs: 0, stepMs: 0 });
    const impl = async (cmd: string) => {
      if (cmd === "system_app_info") return { version: "0.1.0", os: "windows", cacheDir: "C:\\cache" };
      throw { code: "NOT_IMPLEMENTED", message: `${cmd} is not implemented yet` };
    };
    installFallback(
      { backend: mock.backend, deviceBackend: mock.deviceBackend, events: mock.events.on },
      { backend: impl as unknown as Backend, deviceBackend: impl as unknown as DeviceBackend, events: createEmitter().on },
    );
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    await toBinding("SPI");
    await w!.get('[data-binding="phrase"]').setValue(true);
    await w!.get('[data-test="phrase"]').setValue(PHRASE);
    await click(w!, "phrase-apply");
    expect(wizard.model.receiver.elrsUid).toHaveLength(6);
    expect(demoState.simulated).toContain("config_elrs_uid");
    expect(w!.find('[data-test="demo-banner"]').exists()).toBe(true);
  });

  it("is translated (he)", async () => {
    setLocale("he");
    await boot("onboard-elrs");
    await toBinding("SPI");
    expect(w!.text()).toContain("ביטוי צימוד");
  });
});
