// SPDX-License-Identifier: GPL-3.0-or-later
// Test helper: click through every wizard screen to Done on top of a mock backend.
import { flushPromises, type VueWrapper } from "@vue/test-utils";
import { guardFor } from "../src/wizard/guards";
import { nav } from "../src/wizard/nav";
import { wizard } from "../src/wizard/state";

export async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) await flushPromises();
}

export async function click(w: VueWrapper, sel: string): Promise<void> {
  await w.get(`[data-test="${sel}"]`).trigger("click");
  await settle();
}

export interface WalkHooks {
  /** Called on each screen before the default actions (answer screen-specific questions here). */
  onStep?: (id: string) => Promise<void> | void;
}

/** Walk every step to Done; `firmware` picks the keep or the update path. Returns the visited ids. */
export async function walkToDone(w: VueWrapper, firmware: "keep" | "update", hooks: WalkHooks = {}): Promise<string[]> {
  const seen: string[] = [];
  for (let guard = 0; guard < 30 && nav.currentId !== "done"; guard++) {
    const id = nav.currentId;
    seen.push(id);
    await hooks.onStep?.(id);
    if (id === "connect" && !wizard.fc) await click(w, "connect");
    if (id === "firmware" && wizard.ui.firmwareDecision === null) await click(w, firmware);
    if (id === "components" && wizard.model.receiver.kind === "NONE") {
      // Pick something for every optional screen so all of them are visited.
      await w.get('[data-rx-kind="SERIAL"]').setValue(true);
      await w.get('[data-vtx-kind="ANALOG"]').setValue(true);
      await w.get('[data-test="gps"]').setValue(true);
      await settle();
    }
    if (id === "binding") {
      // UART CRSF: say it is ExpressLRS and bind with the button later.
      if (w.find('[data-family="ELRS"]').exists() && !wizard.ui.crsfFamily) await w.get('[data-family="ELRS"]').setValue(true);
      if (w.find('[data-binding="button"]').exists() && !wizard.ui.elrsBinding) await w.get('[data-binding="button"]').setValue(true);
      await settle();
    }
    if (id === "osd" && wizard.osd.mode === null) {
      // Set the OSD up from the recommended layout.
      await w.get('[data-osd-mode="choose"]').setValue(true);
      await settle();
    }
    for (const sel of w.findAll("select[data-role]")) {
      const el = sel.element as HTMLSelectElement;
      const first = [...el.options].find((o) => o.value !== "");
      if (el.value === "" && first) await sel.setValue(first.value);
    }
    await settle();
    if (id === "apply" && wizard.ui.applyStatus !== "verified") await click(w, "apply");
    await click(w, "next");
    if (nav.currentId === id) throw new Error(`stuck on ${id}: ${JSON.stringify(guardFor(id).validate(wizard))}`);
  }
  return seen;
}
