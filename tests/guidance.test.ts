// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { UPCOMING_ERROR_CODES } from "../src/api/upcoming";
import { CURRENT_ERROR_CODES, guidanceFor, HELP_LINKS, toAppError } from "../src/errors/guidance";

describe("error guidance", () => {
  it("has guidance for every current error.rs code and every agreed upcoming code", () => {
    for (const code of [...CURRENT_ERROR_CODES, ...UPCOMING_ERROR_CODES]) {
      const g = guidanceFor(code);
      expect(g.code).toBe(code);
      expect(g.actions.length).toBeGreaterThan(0);
      expect(g.stepKeys.length).toBeGreaterThan(0);
    }
  });

  it("falls back to UNKNOWN for codes it does not know", () => {
    expect(guidanceFor("SOMETHING_NEW").code).toBe("UNKNOWN");
  });

  it("normalises thrown values into the AppError shape", () => {
    expect(toAppError({ code: "PORT_BUSY", message: "busy" })).toEqual({ code: "PORT_BUSY", message: "busy" });
    expect(toAppError(new Error("boom"))).toEqual({ code: "UNKNOWN", message: "boom" });
    expect(toAppError("plain")).toEqual({ code: "UNKNOWN", message: "plain" });
  });

  it("only links to hosts allowed by the opener capability", () => {
    const allowed = ["https://betaflight.com/", "https://github.com/betaflight/", "https://github.com/eligreenmac/betaflight-setup-wizard/"];
    for (const url of Object.values(HELP_LINKS)) expect(allowed.some((p) => url.startsWith(p))).toBe(true);
  });
});
