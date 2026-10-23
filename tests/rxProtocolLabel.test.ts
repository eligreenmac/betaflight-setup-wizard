// SPDX-License-Identifier: GPL-3.0-or-later
// One helper turns receiver protocol ids into friendly names: the receiver step, the
// issue list (rxProtocol param), the firmware step's mid-wizard banner and Review.
import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import IssueList from "../src/components/IssueList.vue";
import { i18n, setLocale } from "../src/i18n";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { rxProtocolLabel, SERIAL_RX_PROTOCOLS, SPI_RX_PROTOCOLS } from "../src/wizard/receiver";

const g = i18n.global as unknown as { t: (k: string) => string; te: (k: string) => boolean };
const t = (k: string) => g.t(k);
const te = (k: string) => g.te(k);

afterEach(() => setLocale("en"));

describe("rxProtocolLabel", () => {
  it("gives the friendly name in en and he", () => {
    setLocale("en");
    expect(rxProtocolLabel("CRSF", t, te)).toBe("CRSF (ExpressLRS, Crossfire, Tracer)");
    expect(rxProtocolLabel("SPEK2048", t, te)).toBe(en.receiver.protocols.SPEK2048);
    setLocale("he");
    expect(rxProtocolLabel("CRSF", t, te)).toBe(he.receiver.protocols.CRSF);
  });

  it("every offered protocol has a name; unknown ids stay as-is (never an i18n key); empty -> ''", () => {
    for (const p of [...SERIAL_RX_PROTOCOLS, ...SPI_RX_PROTOCOLS]) expect(rxProtocolLabel(p, t, te)).not.toContain("receiver.protocols");
    expect(rxProtocolLabel("NEWPROTO", t, te)).toBe("NEWPROTO");
    expect(rxProtocolLabel(null, t, te)).toBe("");
  });
});

describe("IssueList translates the rxProtocol param", () => {
  for (const loc of ["en", "he"] as const) {
    it(loc, () => {
      setLocale(loc);
      const msgs = loc === "en" ? en : he;
      const w = mount(IssueList, {
        global: { plugins: [i18n] },
        props: { issues: [{ key: "validation.rxOptionMissingApplied", params: { rxProtocol: "CRSF" } }, { key: "validation.rxOptionMissingNoUpdate", params: { rxProtocol: "SPEK1024" } }] },
      });
      expect(w.text()).toContain(msgs.validation.rxOptionMissingApplied.replace("{rxProtocol}", msgs.receiver.protocols.CRSF));
      expect(w.text()).toContain(msgs.validation.rxOptionMissingNoUpdate.replace("{rxProtocol}", msgs.receiver.protocols.SPEK1024));
      w.unmount();
    });
  }
});
