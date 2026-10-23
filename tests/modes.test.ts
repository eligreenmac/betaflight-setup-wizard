// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { mockFirmwareCatalog } from "../src/mocks/scenarios";
import { catalogState, resetCatalog } from "../src/api/catalog";
import { displayBoxName, MODES, modeAvailable, modeKeyOf, modeName, normalizeModeNames, usesNewModeNames } from "../src/wizard/switches";
import { validateSwitches } from "../src/wizard/guards";
import { createWizardState, setBoard } from "../src/wizard/state";
import type { FcInfo } from "../src/types/model";

const fc = (firmwareVersion: string): FcInfo => ({ fcVariant: "BTFL", firmwareVersion, apiVersion: "1.46", boardName: "X", manufacturerId: null, mcu: null, uid: null, buildOptions: [], serialPorts: [], rxSpi: { present: null, expresslrsPins: false, protocol: null, featureEnabled: null, resources: [] } });

describe("mode names: stable in the model, labels by firmware", () => {
  it("the model only ever holds the stable config::modes names", () => {
    expect(Object.values(MODES)).toEqual(["ARM", "ANGLE", "HORIZON", "BEEPER", "FLIP OVER AFTER CRASH", "ALTHOLD", "POSHOLD", "GPS RESCUE", "PREARM", "AIR MODE"]);
    for (const fw of ["4.4.3", "4.5.2", "2025.12.0"]) {
      expect(modeName("altHold")).toBe("ALTHOLD");
      expect(modeName("posHold")).toBe("POSHOLD");
      expect(modeName("arm")).toBe("ARM");
      expect(usesNewModeNames(fw)).toBe(fw.startsWith("2025"));
    }
  });

  it("display box names still follow the firmware", () => {
    expect(displayBoxName("altHold", "4.5.2")).toBe("BARO");
    expect(displayBoxName("altHold", "2025.12.0")).toBe("ALTHOLD");
    expect(displayBoxName("posHold", "4.4.3")).toBe("GPS HOLD");
    expect(displayBoxName("posHold", "2025.12.0")).toBe("POS HOLD");
  });

  it("maps stable names, display names and aliases to the same key", () => {
    for (const n of ["ALTHOLD", "BARO", "altitude_hold"]) expect(modeKeyOf(n)).toBe("altHold");
    for (const n of ["POSHOLD", "POS HOLD", "GPS HOLD"]) expect(modeKeyOf(n)).toBe("posHold");
    expect(modeKeyOf("Turtle")).toBe("turtle");
    expect(modeKeyOf("NOPE")).toBeNull();
  });

  it("normalises any stored spelling to the stable name, whatever the firmware", () => {
    const sw = [{ mode: "BARO" }, { mode: "ARM" }, { mode: "POS HOLD" }, { mode: "GPS HOLD" }];
    normalizeModeNames(sw);
    expect(sw.map((s) => s.mode)).toEqual(["ALTHOLD", "ARM", "POSHOLD", "POSHOLD"]);
    const s = createWizardState();
    s.model.switches.push({ mode: "POS HOLD", auxChannel: 2, rangeMin: 1700, rangeMax: 2100 });
    setBoard(s, fc("2025.12.0"), [], "COM5");
    expect(s.model.switches.map((x) => x.mode)).toEqual(["ARM", "POSHOLD"]);
    setBoard(s, fc("4.5.1"), [], "COM5");
    expect(s.model.switches.map((x) => x.mode)).toEqual(["ARM", "POSHOLD"]); // never renamed per firmware
  });

  it("altitude / position hold need 2025.12 (config::modes does not manage BARO / GPS HOLD)", () => {
    expect(modeAvailable("altHold", "4.5.2")).toBe(false);
    expect(modeAvailable("altHold", "2025.12.0")).toBe(true);
    expect(modeAvailable("angle", "4.4.3")).toBe(true);
    const s = createWizardState();
    setBoard(s, fc("4.5.1"), [], "COM5");
    s.model.switches.push({ mode: "ALTHOLD", auxChannel: 2, rangeMin: 1700, rangeMax: 2100 });
    expect(validateSwitches(s).map((i) => i.key)).toContain("validation.modeNeedsFirmware");
    setBoard(s, fc("2025.12.0"), [], "COM5");
    expect(validateSwitches(s)).toEqual([]);
    s.model.switches.push({ mode: "MAG", auxChannel: 3, rangeMin: 1700, rangeMax: 2100 });
    expect(validateSwitches(s).map((i) => i.key)).toContain("validation.modeUnknown");
  });

  it("with the firmware catalog, its mode list decides availability", () => {
    const cat45 = mockFirmwareCatalog("4.5.1").modes;
    expect(modeAvailable("altHold", "4.5.1", cat45)).toBe(false);
    expect(modeAvailable("gpsRescue", "4.5.1", cat45)).toBe(true);
    expect(modeAvailable("airMode", "4.5.1", cat45)).toBe(true);
    expect(modeAvailable("altHold", "2025.12.0", mockFirmwareCatalog("2025.12.0").modes)).toBe(true);
    // A catalog without PREARM: not offered, and refused if present.
    expect(modeAvailable("prearm", "4.5.1", ["ARM", "ANGLE"])).toBe(false);
    expect(modeAvailable("prearm", "4.5.1", [])).toBe(true); // empty = built-in rules
    try {
      catalogState.catalog = { beepers: [], beaconEvents: [], osdElements: [], modes: ["ARM", "ANGLE"] };
      const s = createWizardState();
      s.model.switches.push({ mode: "PREARM", auxChannel: 3, rangeMin: 1700, rangeMax: 2100 });
      expect(validateSwitches(s).map((i) => i.key)).toContain("validation.modeNotInFirmware");
    } finally {
      resetCatalog();
    }
  });
});
