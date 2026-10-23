// SPDX-License-Identifier: GPL-3.0-or-later
// UI tests: the app shell and the step screens on top of the scenario mock.
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "../src/App.vue";
import { applyDirection, i18n, setLocale } from "../src/i18n";
import { installMock, uninstallMock } from "../src/mocks/install";
import type { Scenario, ScenarioMock } from "../src/mocks/scenarios";
import { motorTest } from "../src/wizard/motorTest";
import { nav, resetNav } from "../src/wizard/nav";
import { resetWizard, wizard } from "../src/wizard/state";

let mock: ScenarioMock;
let w: VueWrapper | null = null;

async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) await flushPromises();
}
async function boot(scenario: Scenario = "happy"): Promise<VueWrapper> {
  mock = installMock({ scenario, latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  return w;
}
async function next(): Promise<void> {
  await w!.get('[data-test="next"]').trigger("click");
  await settle();
}
/** Welcome → Connect → connect → Firmware (keep) → Components. */
async function toComponents(): Promise<void> {
  await next();
  expect(nav.currentId).toBe("connect");
  await w!.get('[data-test="connect"]').trigger("click");
  await settle();
  expect(w!.find('[data-test="connected"]').exists()).toBe(true);
  await next();
  expect(nav.currentId).toBe("firmware");
  await w!.get('[data-test="keep"]').trigger("click");
  await next();
  expect(nav.currentId).toBe("components");
}

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

describe("app shell", () => {
  it("Hebrew sets dir=rtl and lang on <html>, English ltr", () => {
    setLocale("he");
    expect(document.documentElement.dir).toBe("rtl");
    expect(document.documentElement.lang).toBe("he");
    setLocale("en");
    applyDirection("en");
    expect(document.documentElement.dir).toBe("ltr");
  });

  it("Next is gated by the step guard and shows why", async () => {
    await boot();
    await next(); // welcome → connect
    await next(); // not connected: stays
    expect(nav.currentId).toBe("connect");
    expect(w!.text()).toContain("Connect a flight controller first.");
  });

  it("walks connect → firmware → components with the mock board", async () => {
    await boot();
    await toComponents();
    expect(w!.text()).toContain("What is connected?");
  });
});

describe("steps", () => {
  it("receiver: CRSF asks the telemetry question (default yes) and the port picker is inside the step", async () => {
    await boot();
    await toComponents();
    await w!.get('[data-rx-kind="SERIAL"]').setValue(true);
    await next();
    expect(nav.currentId).toBe("receiver");
    await w!.get('[data-protocol="CRSF"]').setValue(true);
    await settle();
    expect(w!.text()).toContain("Send telemetry");
    expect(wizard.model.receiver.telemetry).toBe(true);
    expect(w!.find("select").exists()).toBe(true);
  });

  it("esc: motor buttons stay disabled until BOTH acknowledgements are ticked", async () => {
    await boot();
    await toComponents();
    nav.currentId = "esc";
    await settle();
    expect(wizard.ui.motorTestAvailable).toBe(true);
    const motor = () => w!.get('[data-motor="0"]');
    expect(motor().attributes("disabled")).toBeDefined();
    await w!.get('[data-test="ack-props"]').setValue(true);
    expect(motor().attributes("disabled")).toBeDefined();
    await w!.get('[data-test="ack-battery"]').setValue(true);
    expect(motor().attributes("disabled")).toBeUndefined();
    await motor().trigger("click");
    await settle();
    expect(motorTest.state.running).toBe(true);
    // Withdrawing an acknowledgement stops the motors.
    await w!.get('[data-test="ack-battery"]').setValue(false);
    await settle();
    expect(motorTest.state.active).toBe(false);
    expect(mock.device.motorsSpinning.size).toBe(0);
  });

  it("esc: leaving the step stops a running motor test", async () => {
    await boot();
    await toComponents();
    nav.currentId = "esc";
    await settle();
    wizard.ui.propsOffAck = true;
    wizard.ui.batteryOffAck = true;
    await settle();
    await w!.get('[data-motor="2"]').trigger("click");
    await settle();
    expect(motorTest.state.running).toBe(true);
    nav.currentId = "switches";
    await settle();
    expect(motorTest.state.active).toBe(false);
    expect(mock.device.motorsSpinning.size).toBe(0);
  });

  it("esc: battery-powered checks are a disabled phase-2 stub with a warning", async () => {
    await boot();
    await toComponents();
    nav.currentId = "esc";
    await settle();
    expect(w!.text()).toContain("These checks need the battery connected");
    const stubs = w!.findAll(".disabled-stub input");
    expect(stubs.length).toBeGreaterThan(0);
    for (const s of stubs) expect(s.attributes("disabled")).toBeDefined();
  });

  it("esc: legacy backend without motor commands shows the unavailable fallback", async () => {
    await boot("legacy-backend");
    await toComponents();
    nav.currentId = "esc";
    await settle();
    expect(wizard.ui.motorTestAvailable).toBe(false);
    expect(w!.find('[data-test="motor-test-unavailable"]').exists()).toBe(true);
  });

  it("review lists clearings first, then apply verifies", async () => {
    await boot();
    await toComponents();
    nav.currentId = "review";
    await settle();
    const html = w!.html();
    expect(w!.find('[data-test="clearings"]').exists()).toBe(true);
    expect(html.indexOf('data-test="clearings"')).toBeLessThan(html.indexOf('data-test="cli"'));
    expect(wizard.ui.script).not.toBeNull();
    await next();
    expect(nav.currentId).toBe("apply");
    await w!.get('[data-test="apply"]').trigger("click");
    await settle();
    expect(wizard.ui.applyStatus).toBe("verified");
    expect(w!.find('[data-test="verified"]').exists()).toBe(true);
  });

  it("apply shows a mismatch table with key / expected / actual", async () => {
    await boot("verify-mismatch");
    await toComponents();
    nav.currentId = "review";
    await settle();
    nav.currentId = "apply";
    await settle();
    await w!.get('[data-test="apply"]').trigger("click");
    await settle();
    expect(wizard.ui.applyStatus).toBe("mismatch");
    const table = w!.get('[data-test="mismatches"] table');
    expect(table.text()).toContain("serialrx_provider");
    expect(table.text()).toContain("CRSF");
  });
});
