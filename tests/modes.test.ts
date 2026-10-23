// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { modeKeyOf, modeName, renameModesForFirmware, usesNewModeNames } from "../src/wizard/switches";
import { createWizardState, setBoard } from "../src/wizard/state";
import type { FcInfo } from "../src/types/model";

describe("mode names by firmware version", () => {
  it("uses the 4.x names before 2025.12 and the renamed boxes from 2025.12", () => {
    expect(usesNewModeNames("4.5.2")).toBe(false);
    expect(usesNewModeNames("2025.12.0")).toBe(true);
    expect(usesNewModeNames("2026.6.1")).toBe(true);
    expect(modeName("altHold", "4.5.2")).toBe("BARO");
    expect(modeName("altHold", "2025.12.0")).toBe("ALTHOLD");
    expect(modeName("posHold", "4.4.3")).toBe("GPS HOLD");
    expect(modeName("posHold", "2025.12.0")).toBe("POS HOLD");
    expect(modeName("arm", "2025.12.0")).toBe("ARM");
  });

  it("maps names from either generation back to the same key", () => {
    expect(modeKeyOf("BARO")).toBe("altHold");
    expect(modeKeyOf("ALTHOLD")).toBe("altHold");
    expect(modeKeyOf("GPS HOLD")).toBe("posHold");
    expect(modeKeyOf("POS HOLD")).toBe("posHold");
    expect(modeKeyOf("NOPE")).toBeNull();
  });

  it("renames stored switches when the board's firmware changes (e.g. after flashing)", () => {
    const sw = [{ mode: "BARO" }, { mode: "ARM" }, { mode: "GPS HOLD" }];
    renameModesForFirmware(sw, "2025.12.0");
    expect(sw.map((s) => s.mode)).toEqual(["ALTHOLD", "ARM", "POS HOLD"]);
    const s = createWizardState();
    s.model.switches.push({ mode: "ALTHOLD", auxChannel: 2, rangeMin: 1700, rangeMax: 2100 });
    const fc: FcInfo = { fcVariant: "BTFL", firmwareVersion: "4.5.1", apiVersion: "1.46", boardName: "X", manufacturerId: null, mcu: null, uid: null, buildOptions: [] };
    setBoard(s, fc, [], "COM5");
    expect(s.model.switches.map((x) => x.mode)).toEqual(["ARM", "BARO"]);
  });
});
