// SPDX-License-Identifier: GPL-3.0-or-later
// Receiver option missing while the mixed-backend guard forbids a firmware update: the
// generic "can't update right now" wording, and another protocol is the way forward.
import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../src/api/fallback", async (orig) => {
  const real = await orig<typeof import("../src/api/fallback")>();
  // flasher_flash would mix a simulated answer with the real board.
  return { ...real, writeBlock: (cmd: string) => (cmd === "flasher_flash" ? { simulated: ["flasher_required_options"], real: ["device_connect"] } : null) };
});

import App from "../src/App.vue";
import { resetDemoState } from "../src/api/fallback";
import { i18n, setLocale } from "../src/i18n";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { installMock, uninstallMock } from "../src/mocks/install";
import { guards } from "../src/wizard/guards";
import { nav, resetNav } from "../src/wizard/nav";
import { reflashBlock } from "../src/wizard/reflash";
import { resetWizard, setReceiverKind, setReceiverProtocol, wizard } from "../src/wizard/state";
import { click, settle } from "./walk";

let w: VueWrapper | null = null;
beforeEach(() => {
  resetDemoState();
  resetWizard();
  resetNav();
  setLocale("en");
});
afterEach(() => {
  w?.unmount();
  w = null;
  uninstallMock();
  document.body.innerHTML = "";
});

async function toReceiver(): Promise<void> {
  installMock({ scenario: "rx-option-missing", latencyMs: 0, stepMs: 0 });
  w = mount(App, { global: { plugins: [i18n] }, attachTo: document.body });
  await settle();
  await click(w, "next");
  await click(w, "connect");
  setReceiverKind(wizard, "SERIAL");
  setReceiverProtocol(wizard, "CRSF");
  nav.currentId = "receiver";
  await settle();
}

describe("receiver option missing, update blocked by the mixed-backend guard", () => {
  it("generic wording, no offer, Next blocked until another protocol is picked", async () => {
    await toReceiver();
    expect(reflashBlock(wizard)).toBe("mixed");
    expect(w!.get('[data-test="rx-option-no-update"]').text()).toBe(en.receiver.optionMissingNoUpdate.replaceAll("{protocol}", "CRSF"));
    expect(w!.get('[data-test="rx-option-missing"]').text()).not.toContain(en.receiver.optionMissingChoices);
    expect(w!.find('[data-test="reflash-offer"]').exists()).toBe(false);
    await click(w!, "next");
    expect(nav.currentId).toBe("receiver");
    expect(w!.text()).toContain(en.validation.rxOptionMissingNoUpdate.replace("{protocol}", "CRSF"));
    await w!.get('[data-protocol="SBUS"]').setValue(true);
    await settle();
    expect(guards.receiver.validate(wizard).map((i) => i.key).filter((k) => k.startsWith("validation.rxOption"))).toEqual([]);
  });

  it("in Hebrew", async () => {
    setLocale("he");
    await toReceiver();
    expect(w!.get('[data-test="rx-option-no-update"]').text()).toBe(he.receiver.optionMissingNoUpdate.replaceAll("{protocol}", "CRSF"));
  });
});
