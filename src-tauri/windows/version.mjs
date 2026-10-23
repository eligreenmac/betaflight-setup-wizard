// SPDX-License-Identifier: GPL-3.0-or-later
// Windows numeric version (VERSIONINFO FILEVERSION/PRODUCTVERSION) for the app version,
// derived the same way Tauri's WiX bundler derives the MSI ProductVersion:
//   X.Y.Z-N -> X.Y.Z.N   (numeric pre-release N <= 65535)
//   X.Y.Z   -> X.Y.Z.0   (the MSI keeps X.Y.Z; a VERSIONINFO always has 4 fields)
// Returns null for anything else (.github/scripts/check-release-version.mjs explains why).
export function windowsNumericVersion(version) {
  const m = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(0|[1-9]\d*))?$/.exec(version ?? "");
  if (!m) return null;
  const [maj, min, pat, pre = 0] = m.slice(1).map((x) => (x === undefined ? undefined : Number(x)));
  if (maj > 255 || min > 255 || pat > 65535 || pre > 65535) return null;
  return `${maj}.${min}.${pat}.${pre}`;
}
