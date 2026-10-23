// SPDX-License-Identifier: GPL-3.0-or-later
// compareVersions follows semver: a pre-release (-alpha, -beta, -RC1) sorts below its release.
// versionAtLeast is the feature gate: a pre-release of X counts as X.
import { describe, expect, it } from "vitest";
import { compareVersions, isSupportedFirmware, versionAtLeast } from "../src/wizard/version";
import { modeAvailable, usesNewModeNames } from "../src/wizard/switches";

describe("compareVersions (semver order)", () => {
  it("numeric core", () => {
    expect(compareVersions("4.10.0", "4.9.2")).toBe(1);
    expect(compareVersions("2025.12.0", "4.5.2")).toBe(1);
    expect(compareVersions("4.5", "4.5.0")).toBe(0);
    expect(compareVersions("4.5.1", "4.5.2")).toBe(-1);
  });

  it("a pre-release sorts below its release, above the previous one", () => {
    expect(compareVersions("2026.12.0-alpha", "2026.12.0")).toBe(-1);
    expect(compareVersions("2026.12.0", "2026.12.0-alpha")).toBe(1);
    expect(compareVersions("2025.12.0-RC1", "2025.12.0")).toBe(-1);
    expect(compareVersions("2026.12.0-alpha", "2026.6.2")).toBe(1);
    expect(compareVersions("2026.12.0-RC1", "2026.12.0-RC1")).toBe(0);
    expect(compareVersions("2026.12.0-rc1", "2026.12.0-RC1")).toBe(0);
  });

  it("orders pre-releases alpha < beta < RC, numerically within (RC2 < RC10), build metadata ignored", () => {
    const sorted = ["2026.12.0", "2026.12.0-RC10", "2026.12.0-beta", "2026.12.0-RC2", "2026.12.0-alpha", "2026.12.0-RC1", "2026.6.2"].sort(compareVersions);
    expect(sorted).toEqual(["2026.6.2", "2026.12.0-alpha", "2026.12.0-beta", "2026.12.0-RC1", "2026.12.0-RC2", "2026.12.0-RC10", "2026.12.0"]);
    expect(compareVersions("4.5.2+abc123", "4.5.2")).toBe(0);
  });
});

describe("versionAtLeast (feature gates)", () => {
  it("a pre-release of the first version with a feature has it", () => {
    expect(versionAtLeast("2025.12.0-RC1", "2025.12.0")).toBe(true);
    expect(versionAtLeast("2025.12.0", "2025.12.0")).toBe(true);
    expect(versionAtLeast("4.5.2", "2025.12.0")).toBe(false);
    expect(usesNewModeNames("2025.12.0-RC1")).toBe(true);
    expect(modeAvailable("altHold", "2025.12.0-RC1")).toBe(true);
    expect(isSupportedFirmware("BTFL", "4.4.0-RC1")).toBe(true);
    expect(isSupportedFirmware("BTFL", "4.3.2")).toBe(false);
  });
});
