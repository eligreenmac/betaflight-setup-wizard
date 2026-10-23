// SPDX-License-Identifier: GPL-3.0-or-later
/** Compare dotted versions numerically ("4.10.0" > "4.9.2", "2025.12.0" > "4.5.2"). Returns -1, 0 or 1. */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(/[.-]/).map((x) => parseInt(x, 10) || 0);
  const pb = b.split(/[.-]/).map((x) => parseInt(x, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d > 0 ? 1 : -1;
  }
  return 0;
}

/** Oldest Betaflight the config engine has a dialect for (docs/ARCHITECTURE.md §5). */
export const MIN_SUPPORTED_FIRMWARE = "4.4.0";

export function isSupportedFirmware(fcVariant: string, version: string): boolean {
  return fcVariant === "BTFL" && compareVersions(version, MIN_SUPPORTED_FIRMWARE) >= 0;
}
