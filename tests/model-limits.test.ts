import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  CRAFT_NAME_MAX_LENGTH,
  CRAFT_NAME_PATTERN,
  VTX_BAND_MAX,
  VTX_CHANNEL_MAX,
  VTX_POWER_MAX,
} from "../src/types/model";

const read = (p: string) => JSON.parse(readFileSync(new URL(p, import.meta.url), "utf8"));
const schema = read("../schemas/drone-model.schema.json");
const cases: { valid: string[]; invalid: string[] } = read("../schemas/fixtures/craft-name-cases.json");

describe("model limits mirror the schema (and Rust)", () => {
  const craft = schema.properties.extras.properties.craftName;
  // Ajv-style: JSON-Schema patterns are ECMA regexes with the u flag.
  const fromSchema = new RegExp(craft.pattern, "u");

  it("craftName pattern is the same rule", () => {
    expect(CRAFT_NAME_PATTERN.source).toBe(craft.pattern);
    expect(craft.maxLength).toBe(CRAFT_NAME_MAX_LENGTH);
    for (const c of cases.valid) expect([c, fromSchema.test(c)]).toEqual([c, true]);
    for (const c of cases.invalid) expect([c, fromSchema.test(c)]).toEqual([c, false]);
    // Same exhaustive single-char check as model.rs: only 0x21..0x7E minus # ? - alone.
    for (let b = 0; b <= 255; b++) {
      const want = b >= 0x21 && b <= 0x7e && !"#?-".includes(String.fromCharCode(b));
      expect([b, fromSchema.test(String.fromCharCode(b))]).toEqual([b, want]);
    }
  });

  it("vtx band/channel/power ranges", () => {
    const vtx = schema.properties.vtx.properties;
    expect([vtx.band.minimum, vtx.band.maximum]).toEqual([1, VTX_BAND_MAX]);
    expect([vtx.channel.minimum, vtx.channel.maximum]).toEqual([1, VTX_CHANNEL_MAX]);
    expect([vtx.power.minimum, vtx.power.maximum]).toEqual([1, VTX_POWER_MAX]);
  });
});
