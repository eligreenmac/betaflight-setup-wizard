// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { ALL_ERROR_CODES, DESIGN_ERROR_CODES, ERROR_CATEGORIES, LEGACY_CODE_ALIASES, toUiError, uiError } from "../src/errors/codes";
import { guidanceFor, HELP_LINKS } from "../src/errors/guidance";

describe("error codes (DESIGN.md §6)", () => {
  it("has exactly the 31 design codes, each with a valid category", () => {
    expect(DESIGN_ERROR_CODES).toHaveLength(31);
    expect(new Set(DESIGN_ERROR_CODES).size).toBe(31);
    for (const c of DESIGN_ERROR_CODES) expect(ERROR_CATEGORIES).toContain(uiError(c, "").category);
    expect(uiError("VERIFY_MISMATCH", "x")).toEqual({ code: "VERIFY_MISMATCH", category: "Flash", message: "x" });
    expect(uiError("MOTOR_TEST_REFUSED", "x").category).toBe("Safety");
  });

  it("normalises UiError, Phase 0 AppError codes and raw values", () => {
    expect(toUiError({ code: "PORT_BUSY", category: "Connection", message: "busy" })).toEqual({ code: "PORT_BUSY", category: "Connection", message: "busy" });
    // category is filled in when the backend does not send one yet
    expect(toUiError({ code: "BOARD_ARMED", message: "armed" }).category).toBe("Safety");
    expect(toUiError({ code: "SERIAL", message: "x" })).toEqual({ code: "SERIAL_ERROR", category: "Connection", message: "x" });
    expect(toUiError({ code: "NOT_IMPLEMENTED", message: "stub" }).code).toBe("NOT_IMPLEMENTED");
    expect(toUiError("Command motor_test_start not found").code).toBe("NOT_IMPLEMENTED");
    expect(toUiError({ code: "SOMETHING_NEW", message: "m" })).toMatchObject({ code: "UNKNOWN", message: "SOMETHING_NEW: m" });
    expect(toUiError(new Error("boom"))).toEqual({ code: "UNKNOWN", category: "Operation", message: "boom" });
    for (const target of Object.values(LEGACY_CODE_ALIASES)) expect(ALL_ERROR_CODES).toContain(target);
  });
});

describe("error guidance", () => {
  it("has guidance for every code", () => {
    for (const code of ALL_ERROR_CODES) {
      const g = guidanceFor(code);
      expect(g.code).toBe(code);
      expect(g.stepKeys.length).toBeGreaterThan(0);
    }
    expect(guidanceFor("MOTOR_TEST_REFUSED").showMessage).toBe(true);
    expect(guidanceFor("VERIFY_MISMATCH").actions[0]!.id).toBe("retryFullErase");
  });

  it("maps unknown and legacy codes", () => {
    expect(guidanceFor("SOMETHING_NEW").code).toBe("UNKNOWN");
    expect(guidanceFor("DRIVER").code).toBe("DRIVER_MISSING");
  });

  it("only links to hosts allowed by the opener capability", () => {
    const allowed = ["https://betaflight.com/", "https://github.com/betaflight/", "https://github.com/eligreenmac/betaflight-setup-wizard/"];
    for (const url of Object.values(HELP_LINKS)) expect(allowed.some((p) => url.startsWith(p))).toBe(true);
  });
});
