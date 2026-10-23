// SPDX-License-Identifier: GPL-3.0-or-later
// Safety: a write to the board never mixes simulated (mock) and real data (api/fallback.ts).
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { api, type Backend } from "../src/api/commands";
import { createEmitter } from "../src/api/events";
import { demoState, installFallback, resetDemoState, writeBlock } from "../src/api/fallback";
import { defaultMockHandlers } from "../src/api/mock";
import { i18n, setLocale } from "../src/i18n";
import { uninstallMock } from "../src/mocks/install";
import { createScenarioMock } from "../src/mocks/scenarios";
import { emptyDroneModel, type CliScript, type FcInfo, type FirmwareImage, type FirmwareRequest } from "../src/types/model";
import ApplyStep from "../src/wizard/steps/ApplyStep.vue";
import { resetWizard, wizard } from "../src/wizard/state";

type Impl = (args: unknown) => unknown;
const notImpl = (cmd: string) => ({ code: "NOT_IMPLEMENTED", message: `${cmd} is not implemented yet` });
const fc = (defaultMockHandlers.device_identify as () => FcInfo)();
/** The board half of a real backend: connect and identify work. */
const realBoard: Record<string, Impl> = { device_connect: () => undefined, device_identify: () => fc };
const realScript: CliScript = { commands: ["set gyro_lpf1_static_hz = 250"], warnings: [], preview: "" } as unknown as CliScript;

/** A real backend that implements exactly `impl`; everything else is a Phase 0 stub. */
function setup(impl: Record<string, Impl>) {
  const calls: string[] = [];
  const real = async (cmd: string, args: unknown) => {
    calls.push(cmd);
    if (impl[cmd]) return impl[cmd]!(args);
    throw notImpl(cmd);
  };
  const mock = createScenarioMock({ latencyMs: 0, stepMs: 0 });
  installFallback({ backend: mock.backend, events: mock.events.on }, { backend: real as unknown as Backend, events: createEmitter().on });
  return { calls, mock };
}

async function connect(): Promise<void> {
  await api.device.connect("COM5");
  await api.device.identify();
}

beforeEach(() => {
  resetDemoState();
  resetWizard();
  setLocale("en");
});
afterEach(() => {
  uninstallMock();
  resetDemoState();
});

describe("mock/real write guard: config", () => {
  it("blocks a mocked config_plan from reaching a real config_apply (nothing is sent)", async () => {
    const real = setup({ ...realBoard, config_apply: () => ({ applied: 1, errors: [], saved: true, reconnected: true, backup: null }) });
    await connect();
    const script = await api.config.plan(emptyDroneModel()); // stub -> mock
    expect(demoState.sources.config_plan).toBe("mock");
    expect(writeBlock("config_apply")).toEqual({ simulated: ["config_plan"], real: ["device_connect"] });
    await expect(api.config.apply(script)).rejects.toMatchObject({ code: "MIXED_BACKEND", category: "Safety" });
    expect(real.calls).not.toContain("config_apply");
  });

  it("blocks the reverse: a real plan for a real board is never 'applied' by the mock", async () => {
    const real = setup({ ...realBoard, config_plan: () => realScript });
    await connect();
    const script = await api.config.plan(emptyDroneModel());
    expect(writeBlock("config_apply")).toBeNull(); // board and plan agree; apply's own source is unknown until tried
    await expect(api.config.apply(script)).rejects.toMatchObject({ code: "MIXED_BACKEND" });
    expect(real.calls).toContain("config_apply"); // asked the real backend, which could not do it
    expect(demoState.simulated).not.toContain("config_apply");
    expect(demoState.sources.config_apply).toBeUndefined();
  });

  it("blocks a mocked board with a real plan before anything is sent", async () => {
    const real = setup({ config_plan: () => realScript, config_apply: () => ({ applied: 1, errors: [], saved: true, reconnected: true, backup: null }) });
    await connect(); // stubs -> simulated board
    await api.config.plan(emptyDroneModel()); // real
    await expect(api.config.apply(realScript)).rejects.toMatchObject({ code: "MIXED_BACKEND" });
    expect(real.calls).not.toContain("config_apply");
  });

  it("all simulated: the write stays on the mock and never touches the real backend", async () => {
    const real = setup({ config_apply: () => { throw new Error("must not be called"); } });
    await connect();
    const script = await api.config.plan(emptyDroneModel());
    const res = await api.config.apply(script);
    expect(res.saved).toBe(true);
    expect(real.calls).not.toContain("config_apply");
    expect(demoState.sources.config_apply).toBe("mock");
  });

  it("all real: the write goes to the real backend", async () => {
    const real = setup({ ...realBoard, config_plan: () => realScript, config_apply: () => ({ applied: 1, errors: [], saved: true, reconnected: true, backup: null }) });
    await connect();
    await api.config.apply(await api.config.plan(emptyDroneModel()));
    expect(real.calls).toContain("config_apply");
    expect(demoState.sources.config_apply).toBe("real");
    expect(demoState.simulated).toEqual([]);
  });

  it("forced demo is all mock and never blocked", async () => {
    const real = setup({ ...realBoard, config_apply: () => { throw new Error("must not be called"); } });
    await connect(); // real
    demoState.forced = true;
    expect(writeBlock("config_apply")).toBeNull();
    await api.config.apply(await api.config.plan(emptyDroneModel()));
    expect(real.calls).toEqual(["device_connect", "device_identify"]);
  });
});

describe("mock/real write guard: firmware", () => {
  const request = { target: "STM32F405", release: "4.5.2", options: [] } as unknown as FirmwareRequest;

  it("blocks a mocked firmware download from being flashed to a real board", async () => {
    const real = setup({ ...realBoard, flasher_list_dfu_devices: () => [], flasher_flash: () => undefined });
    await connect();
    const image: FirmwareImage = await api.flasher.fetchFirmware(request); // stub -> mock file
    await expect(api.flasher.flash(image, null, { fullChipErase: false } as never)).rejects.toMatchObject({ code: "MIXED_BACKEND" });
    expect(real.calls).not.toContain("flasher_flash");
  });

  it("blocks a mocked DFU device list before a real read-protection erase, and the reverse", async () => {
    let real = setup({ ...realBoard, flasher_remove_read_protection: () => undefined });
    await connect();
    await api.flasher.listDfuDevices(); // stub -> mock
    await expect(api.flasher.removeReadProtection("MOCKDFU1", true)).rejects.toMatchObject({ code: "MIXED_BACKEND" });
    expect(real.calls).not.toContain("flasher_remove_read_protection");

    resetDemoState();
    real = setup({ ...realBoard, flasher_list_dfu_devices: () => [] });
    await connect();
    await api.flasher.listDfuDevices(); // real
    await expect(api.flasher.removeReadProtection("X", true)).rejects.toMatchObject({ code: "MIXED_BACKEND" });
    expect(demoState.simulated).not.toContain("flasher_remove_read_protection");
  });

  it("real errors are still passed through untouched", async () => {
    setup({ ...realBoard, flasher_fetch_firmware: () => ({ path: "/real.hex" }), flasher_list_dfu_devices: () => [], flasher_flash: () => Promise.reject({ code: "DFU_NOT_FOUND", message: "x" }) });
    await connect();
    const image = await api.flasher.fetchFirmware(request);
    await expect(api.flasher.flash(image, null, {} as never)).rejects.toMatchObject({ code: "DFU_NOT_FOUND" });
  });
});

describe("Apply step shows the block before anything is written", () => {
  for (const locale of ["en", "he"] as const) {
    it(`mocked plan on a real board: no Apply button, guidance instead (${locale})`, async () => {
      setLocale(locale);
      const real = setup({ ...realBoard, config_apply: () => ({ applied: 1, errors: [], saved: true, reconnected: true, backup: null }) });
      await connect();
      wizard.ui.script = await api.config.plan(emptyDroneModel());
      const w = mount(ApplyStep, { props: { stepId: "apply" }, global: { plugins: [i18n] } });
      await flushPromises();
      expect(w.find('[data-test="apply"]').exists()).toBe(false);
      const text = w.get('[data-test="mixed-blocked"]').text();
      expect(text).toContain(i18n.global.t("errors.MIXED_BACKEND.title"));
      expect(text).toContain(i18n.global.t("errors.MIXED_BACKEND.step1"));
      if (locale === "he") expect(text).toContain("שום דבר לא נכתב ללוח");
      else expect(text).toContain("Nothing was written to the board");
      expect(real.calls).not.toContain("config_apply");
      w.unmount();
    });
  }
});
