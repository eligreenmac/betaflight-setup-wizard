// SPDX-License-Identifier: GPL-3.0-or-later
// Per-call demo fallback (api/fallback.ts) and the persistent "Demo mode" banner.
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { api, type Backend } from "../src/api/commands";
import { createEmitter } from "../src/api/events";
import { demoState, installFallback, isDemoMode, resetDemoState } from "../src/api/fallback";
import { i18n, setLocale } from "../src/i18n";
import { uninstallMock } from "../src/mocks/install";
import { createScenarioMock, type ScenarioMock } from "../src/mocks/scenarios";
import { uiError } from "../src/errors/codes";
import { motorTest } from "../src/wizard/motorTest";
import { guardFor } from "../src/wizard/guards";
import { nav, resetNav } from "../src/wizard/nav";
import { resetWizard, wizard } from "../src/wizard/state";

/** Today's Phase 0 app: only system_app_info is real; the rest are stubs or not registered. */
function phase0(overrides: Record<string, (args: unknown) => unknown> = {}) {
  const calls: string[] = [];
  const impl = async (cmd: string, args: unknown) => {
    calls.push(cmd);
    if (overrides[cmd]) return overrides[cmd]!(args);
    if (cmd === "system_app_info") return { version: "0.1.0", os: "windows", cacheDir: "C:\\cache" };
    if (cmd.startsWith("device_motor") || cmd.startsWith("device_rc") || cmd === "device_probe_board") throw `Command ${cmd} not found`;
    throw { code: "NOT_IMPLEMENTED", message: `${cmd} is not implemented yet` };
  };
  const events = createEmitter();
  return { backend: impl as unknown as Backend, events: events.on, calls };
}

let mock: ScenarioMock;
let w: VueWrapper | null = null;
const settle = async () => {
  for (let i = 0; i < 12; i++) await flushPromises();
};
function setup(real = phase0()) {
  mock = createScenarioMock({ latencyMs: 0, stepMs: 0 });
  installFallback({ backend: mock.backend, events: mock.events.on }, real);
  return real;
}

beforeEach(() => {
  resetDemoState();
  resetWizard();
  resetNav();
  setLocale("en");
});
afterEach(async () => {
  await motorTest.stop("unmount");
  w?.unmount();
  w = null;
  uninstallMock();
  resetDemoState();
});

describe("per-call mock fallback", () => {
  it("uses the real backend when it answers, the mock only for missing commands", async () => {
    const real = setup();
    expect(await api.system.appInfo()).toMatchObject({ os: "windows" }); // real
    expect(isDemoMode()).toBe(false);
    expect(await api.device.listPorts()).toEqual(expect.arrayContaining([expect.objectContaining({ path: "COM5" })])); // NOT_IMPLEMENTED -> mock
    await api.device.motorTestStop(); // "Command ... not found" -> mock
    expect(demoState.simulated).toEqual(["device_list_ports", "device_motor_test_stop"]);
    expect(isDemoMode()).toBe(true);
    expect(real.calls).toEqual(["system_app_info", "device_list_ports", "device_motor_test_stop"]);
  });

  it("never hides a real error behind the mock", async () => {
    setup(phase0({ device_connect: () => Promise.reject(uiError("PORT_BUSY", "COM5 busy")) }));
    await expect(api.device.connect("COM5")).rejects.toMatchObject({ code: "PORT_BUSY" });
    expect(demoState.simulated).toEqual([]);
  });

  it("forced demo sends every call to the mock", async () => {
    const real = setup(phase0({ device_list_ports: () => [] }));
    expect(await api.device.listPorts()).toEqual([]);
    demoState.forced = true;
    expect((await api.device.listPorts()).length).toBeGreaterThan(0);
    expect(real.calls).toEqual(["device_list_ports"]);
  });
});

async function next(): Promise<void> {
  await w!.get('[data-test="next"]').trigger("click");
  await settle();
}
async function click(sel: string): Promise<void> {
  await w!.get(`[data-test="${sel}"]`).trigger("click");
  await settle();
}

/** Walk every step to Done; `firmware` picks the keep or the update path. */
async function walkToDone(firmware: "keep" | "update"): Promise<string[]> {
  const seen: string[] = [];
  for (let guard = 0; guard < 30 && nav.currentId !== "done"; guard++) {
    const id = nav.currentId;
    seen.push(id);
    if (id === "connect" && !wizard.fc) await click("connect");
    if (id === "firmware" && wizard.ui.firmwareDecision === null) {
      if (firmware === "update") {
        await click("update");
        await settle();
      } else await click("keep");
    }
    if (id === "components" && wizard.model.receiver.kind === "NONE") {
      // Pick something for every optional screen so all of them are visited.
      await w!.get('[data-rx-kind="SERIAL"]').setValue(true);
      await w!.get('[data-vtx-kind="ANALOG"]').setValue(true);
      await w!.get('[data-test="gps"]').setValue(true);
      await settle();
    }
    for (const sel of w!.findAll("select[data-role]")) {
      const el = sel.element as HTMLSelectElement;
      const first = [...el.options].find((o) => o.value !== "");
      if (el.value === "" && first) await sel.setValue(first.value);
    }
    await settle();
    if (id === "apply" && wizard.ui.applyStatus !== "verified") await click("apply");
    await next();
    if (nav.currentId === id) throw new Error(`stuck on ${id}: ${JSON.stringify(guardFor(id).validate(wizard))}`);
  }
  return seen;
}

describe("demo mode end to end (no board connected)", () => {
  for (const path of ["keep", "update"] as const) {
    it(`Phase 0 app: banner is shown and every screen reaches Done (firmware: ${path})`, async () => {
      setup();
      w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
      await settle();
      const seen = await walkToDone(path);
      expect(nav.currentId).toBe("done");
      expect(seen).toEqual(["welcome", "connect", "firmware", "components", "receiver", "vtx", "gps", "esc", "switches", "review", "apply"]);
      const banner = w.get('[data-test="demo-banner"]');
      expect(banner.text()).toContain("Demo mode.");
      expect(banner.text()).toContain("Some features are simulated");
      expect(demoState.simulated).toContain("config_apply");
    });
  }

  it("a real backend that finds no board: 'Try the demo' switches everything to the simulated board", async () => {
    setup(phase0({ device_list_ports: () => [], flasher_list_dfu_devices: () => [], system_driver_status: () => ({ dfuWinusb: true, vcp: true, notes: [], devices: [], guidance: [] }) }));
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    await next();
    await click("connect");
    expect(wizard.fc).toBeNull();
    expect(w.find('[data-test="demo-banner"]').exists()).toBe(false); // nothing simulated yet
    await click("use-demo");
    expect(wizard.fc).not.toBeNull();
    expect(w.get('[data-test="demo-banner"]').text()).toContain("Everything runs on a simulated flight controller");
    await walkToDone("keep");
    expect(nav.currentId).toBe("done");
  });

  it("the banner is translated (he)", async () => {
    setLocale("he");
    setup();
    w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
    await settle();
    await next();
    await click("connect");
    expect(w.get('[data-test="demo-banner"]').text()).toContain("מצב הדגמה");
  });
});
