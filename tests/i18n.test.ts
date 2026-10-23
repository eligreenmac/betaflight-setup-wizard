// SPDX-License-Identifier: GPL-3.0-or-later
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { APPLY_STAGES, SERIAL_FUNCTIONS } from "../src/wizard/plan";
import { ALL_ERROR_CODES } from "../src/errors/codes";
import { guidanceFor } from "../src/errors/guidance";
import { isRtl } from "../src/i18n";
import en from "../src/i18n/locales/en.json";
import he from "../src/i18n/locales/he.json";
import { ALL_STEPS } from "../src/wizard/engine";
import { UPDATE_STAGES } from "../src/wizard/flash";
import { PORT_ROLES } from "../src/wizard/ports";
import { SERIAL_RX_PROTOCOLS, SPI_RX_PROTOCOLS } from "../src/wizard/receiver";
import { MODE_KEYS, SWITCH_POSITIONS } from "../src/wizard/switches";

function flat(o: object, p = ""): Record<string, string> {
  return Object.fromEntries(
    Object.entries(o).flatMap(([k, v]) => (v && typeof v === "object" ? Object.entries(flat(v, `${p}${k}.`)) : [[`${p}${k}`, String(v)]])),
  );
}
const EN = flat(en);
const HE = flat(he);
const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? sourceFiles(p) : /\.(ts|vue)$/.test(f) ? [p] : [];
  });
}
const SRC = sourceFiles(join(__dirname, "../src")).map((f) => readFileSync(f, "utf8"));

describe("i18n", () => {
  it("no 'USB power only' / 'battery disconnected' motor-test wording remains (en, he)", () => {
    const banned = [/usb power only/i, /usb only/i, /battery (is )?disconnected/i, /battery unplugged/i, /do not connect the battery/i, /USB בלבד/, /הסוללה מנותקת/, /אל תחברו את הסוללה/];
    for (const [k, v] of [...Object.entries(EN), ...Object.entries(HE)]) for (const re of banned) expect(re.test(v), `${k}: ${v}`).toBe(false);
    expect(EN["esc.motorTest.ackBattery"]).toBe("The battery is connected.");
    expect(HE["esc.motorTest.ackBattery"]).toBe("הסוללה מחוברת.");
    expect(Object.keys(EN).some((k) => /usbOnly|batteryChecks/.test(k))).toBe(false);
  });

  it("he and en have the same keys", () => {
    expect(Object.keys(HE).sort()).toEqual(Object.keys(EN).sort());
  });

  it("he and en use the same {x} placeholders", () => {
    for (const k of Object.keys(EN)) expect(placeholders(HE[k] ?? ""), k).toEqual(placeholders(EN[k]!));
  });

  it("no message uses vue-i18n special syntax (@ linked messages, | plurals, {{x}})", () => {
    for (const [k, v] of [...Object.entries(EN), ...Object.entries(HE)]) {
      expect(v, k).not.toMatch(/[@|$]|\{\{/);
      expect(v.trim(), k).not.toBe("");
    }
  });

  it("hebrew is RTL", () => {
    expect(isRtl("he")).toBe(true);
    expect(isRtl("en")).toBe(false);
  });

  it("every static t('…') key used in src exists", () => {
    const used = new Set<string>();
    for (const s of SRC) for (const m of s.matchAll(/\bt\(\s*["'`]([a-zA-Z][\w.]*)["'`]/g)) used.add(m[1]!);
    for (const s of SRC) for (const m of s.matchAll(/\?\s*["']([a-z]+\.[\w.]+)["']\s*:\s*["']([a-z]+\.[\w.]+)["']/g)) used.add(m[1]!).add(m[2]!);
    expect(used.size).toBeGreaterThan(20);
    expect([...used].filter((k) => !(k in EN))).toEqual([]);
  });

  it("every validation key in guards.ts exists", () => {
    const guards = readFileSync(join(__dirname, "../src/wizard/guards.ts"), "utf8");
    const keys = [...guards.matchAll(/"(validation\.\w+)"/g)].map((m) => m[1]!);
    expect(keys.length).toBeGreaterThan(15);
    expect(keys.filter((k) => !(k in EN))).toEqual([]);
  });

  it("every error code has guidance copy and action labels", () => {
    for (const code of ALL_ERROR_CODES) {
      const g = guidanceFor(code);
      for (const k of [g.titleKey, g.bodyKey, ...g.stepKeys, ...g.actions.map((a) => a.labelKey)]) expect(EN, `${code}: ${k}`).toHaveProperty([k]);
      expect(EN, code).not.toHaveProperty([`errors.${code}.step${g.stepKeys.length + 1}`]);
    }
  });

  it("guidance variants (PORT_BUSY held by APP / EXTERNAL) have complete copy", async () => {
    const { GUIDANCE_VARIANTS } = await import("../src/errors/guidance");
    for (const [v, { steps }] of Object.entries(GUIDANCE_VARIANTS)) {
      for (const k of ["title", "body", ...Array.from({ length: steps }, (_, i) => `step${i + 1}`)]) expect(EN, v).toHaveProperty([`errors.${v}.${k}`]);
      expect(EN, v).not.toHaveProperty([`errors.${v}.step${steps + 1}`]);
    }
    expect(EN["errors.PORT_BUSY_APP.body"]).toMatch(/another window of this app/i);
    expect(EN["errors.PORT_BUSY_EXTERNAL.step1"]).toMatch(/Betaflight Configurator, Cura/);
  });

  it("dynamic key families are complete", () => {
    const need = [
      ...ALL_STEPS.map((s) => s.titleKey),
      ...APPLY_STAGES.map((s) => `apply.stages.${s}`),
      ...UPDATE_STAGES.map((s) => `firmware.stages.${s}`),
      ...["DOWNLOADING", "ENTERING_BOOTLOADER", "WAITING_FOR_DEVICE", "ERASING", "WRITING", "VERIFYING", "REBOOTING", "DONE", "FAILED"].map((s) => `firmware.flashStage.${s}`),
      ...PORT_ROLES.flatMap((r) => [`roles.${r}`, `ports.question.${r}`, `ports.wiring.${r}`]),
      ...["reserved_usb", "softserial_mvp", "assigned_elsewhere", "in_use"].map((r) => `ports.reason.${r}`),
      ...SERIAL_FUNCTIONS.map((f) => `serialFunctions.${f}`),
      ...MODE_KEYS.map((m) => `switches.modes.${m}`),
      ...Object.keys(SWITCH_POSITIONS).map((p) => `switches.positions.${p}`),
      ...[...SERIAL_RX_PROTOCOLS, ...SPI_RX_PROTOCOLS].map((p) => `receiver.protocols.${p}`),
      ...["SERIAL", "SPI", "NONE"].flatMap((k) => [`components.rxKind.${k}`, `components.rxKindHint.${k}`]),
      ...["DIGITAL", "ANALOG", "NONE"].flatMap((k) => [`components.vtxKind.${k}`, `components.vtxKindHint.${k}`]),
      ...["done", "active", "failed"].map((s) => `stageState.${s}`),
    ];
    expect(need.filter((k) => !(k in EN))).toEqual([]);
  });
});
