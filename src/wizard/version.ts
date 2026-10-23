// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * Betaflight version strings: a dotted numeric core ("4.5.2", "2025.12.1") with an optional
 * pre-release suffix ("-alpha", "-beta2", "-RC1"; semver §9) and build metadata ("+abc",
 * ignored, semver §10).
 */
interface ParsedVersion {
  core: number[];
  /** Pre-release identifiers, lower-cased and split into letter / digit runs ("RC10" -> ["rc", 10]). */
  pre: (string | number)[];
}

function parse(v: string): ParsedVersion {
  const s = (v ?? "").trim().replace(/^v/i, "").split("+")[0];
  const dash = s.indexOf("-");
  const coreStr = dash < 0 ? s : s.slice(0, dash);
  const preStr = dash < 0 ? "" : s.slice(dash + 1);
  const core = coreStr.split(".").map((x) => parseInt(x, 10) || 0);
  const pre = (preStr.toLowerCase().match(/[a-z]+|\d+/g) ?? []).map((t) => (/^\d+$/.test(t) ? parseInt(t, 10) : t));
  return { core, pre };
}

const sign = (d: number) => (d > 0 ? 1 : d < 0 ? -1 : 0);

function compareCore(a: number[], b: number[]): number {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d !== 0) return sign(d);
  }
  return 0;
}

/** semver §11: a pre-release is below its release; identifiers compare numerically / lexically, numbers below letters. */
function comparePre(a: (string | number)[], b: (string | number)[]): number {
  if (!a.length || !b.length) return a.length === b.length ? 0 : a.length ? -1 : 1;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i];
    const y = b[i];
    if (x === undefined) return -1; // fewer identifiers = lower ("rc" < "rc1")
    if (y === undefined) return 1;
    if (typeof x === "number" && typeof y === "number") {
      if (x !== y) return sign(x - y);
    } else if (typeof x === "number") return -1;
    else if (typeof y === "number") return 1;
    else if (x !== y) return x < y ? -1 : 1;
  }
  return 0;
}

/**
 * Compare versions (semver order): numeric core first ("4.10.0" > "4.9.2", "2025.12.0" >
 * "4.5.2"), then a pre-release sorts below its release ("2026.12.0-alpha" < "2026.12.0-beta"
 * < "2026.12.0-RC1" < "2026.12.0-RC2" < "2026.12.0"). Returns -1, 0 or 1.
 */
export function compareVersions(a: string, b: string): number {
  const pa = parse(a);
  const pb = parse(b);
  return compareCore(pa.core, pb.core) || comparePre(pa.pre, pb.pre);
}

/**
 * Feature gate: is `version` at least `since`, counting a pre-release of `since` as in?
 * ("2025.12.0-RC1" has what 2025.12.0 has.) Compares the numeric core only; use this for
 * "from firmware X on" checks, compareVersions for ordering releases / "up to date".
 */
export function versionAtLeast(version: string, since: string): boolean {
  return compareCore(parse(version).core, parse(since).core) >= 0;
}

/** Oldest Betaflight the config engine has a dialect for (docs/ARCHITECTURE.md §5). */
export const MIN_SUPPORTED_FIRMWARE = "4.4.0";

export function isSupportedFirmware(fcVariant: string, version: string): boolean {
  return fcVariant === "BTFL" && versionAtLeast(version, MIN_SUPPORTED_FIRMWARE);
}
