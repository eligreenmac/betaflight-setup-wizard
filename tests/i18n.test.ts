import { describe, expect, it } from "vitest";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { isRtl } from "../src/i18n";

function keys(o: object, p = ""): string[] {
  return Object.entries(o).flatMap(([k, v]) =>
    v && typeof v === "object" ? keys(v, `${p}${k}.`) : [`${p}${k}`],
  );
}

describe("i18n", () => {
  it("he and en have the same keys", () => {
    expect(keys(he).sort()).toEqual(keys(en).sort());
  });
  it("hebrew is RTL", () => {
    expect(isRtl("he")).toBe(true);
    expect(isRtl("en")).toBe(false);
  });
});
