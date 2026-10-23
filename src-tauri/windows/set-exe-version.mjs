// SPDX-License-Identifier: GPL-3.0-or-later
// build.beforeBundleCommand (src-tauri/tauri.conf.json): give the compiled app exe the
// same numeric FILEVERSION/PRODUCTVERSION as the installers (X.Y.Z-N -> X.Y.Z.N, see
// version.mjs). tauri-build only puts semver *build metadata* in the 4th field, so it
// writes 0.1.0.0 for 0.1.0-1. The string FileVersion/ProductVersion stay "X.Y.Z-N".
//
// Runs after `cargo build` and before bundling and signing, so the NSIS and MSI
// bundles ship the patched exe. The patch overwrites the 16 version bytes of the
// VS_FIXEDFILEINFO in place (same size, nothing else moves); it is idempotent and
// works on any host. With no Windows exe (Linux/macOS builds) it does nothing.
import { readFileSync, writeFileSync, existsSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { windowsNumericVersion } from "./version.mjs";

const tauriDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const conf = JSON.parse(readFileSync(join(tauriDir, "tauri.conf.json"), "utf8"));

// `tauri build --config ...` (e.g. CI test builds) passes the merged override in TAURI_CONFIG.
let version = conf.version;
try {
  const override = JSON.parse(process.env.TAURI_CONFIG ?? "null");
  if (override && typeof override.version === "string") version = override.version;
} catch {
  /* not JSON: no override */
}
const numeric = windowsNumericVersion(version);
if (!numeric) {
  console.error(`set-exe-version: no Windows numeric version for app version "${version}"`);
  process.exit(1);
}

const pkgName = /^\[package\][^[]*?^name\s*=\s*"([^"]+)"/ms.exec(readFileSync(join(tauriDir, "Cargo.toml"), "utf8"))[1];
const exeName = `${conf.mainBinaryName ?? pkgName}.exe`;
const targetDir = process.env.CARGO_TARGET_DIR ?? join(tauriDir, "target");
const profile = process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
const candidates = [
  process.env.TAURI_ENV_TARGET_TRIPLE && join(targetDir, process.env.TAURI_ENV_TARGET_TRIPLE, profile, exeName),
  join(targetDir, profile, exeName),
].filter((p) => p && existsSync(p));
if (candidates.length === 0) {
  if (process.platform === "win32" || process.env.TAURI_ENV_PLATFORM === "windows") {
    console.error(`set-exe-version: ${exeName} not found under ${targetDir}`);
    process.exit(1);
  }
  console.log(`set-exe-version: no ${exeName} (not a Windows build), nothing to do`);
  process.exit(0);
}
const exe = candidates.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];

function patchFixedFileInfo(buf, numericVersion) {
  // The VERSIONINFO resource: "VS_VERSION_INFO" (UTF-16LE), padding, VS_FIXEDFILEINFO
  // starting with signature 0xFEEF04BD. Require exactly one such resource.
  const key = Buffer.from("VS_VERSION_INFO\0", "utf16le");
  const sig = Buffer.from([0xbd, 0x04, 0xef, 0xfe]);
  const hits = [];
  for (let i = buf.indexOf(key); i !== -1; i = buf.indexOf(key, i + 1)) {
    const s = buf.indexOf(sig, i + key.length);
    if (s !== -1 && s - (i + key.length) <= 4) hits.push(s);
  }
  if (hits.length !== 1) throw new Error(`expected 1 VS_FIXEDFILEINFO, found ${hits.length}`);
  const [a, b, c, d] = numericVersion.split(".").map(Number);
  const ms = ((a << 16) | b) >>> 0;
  const ls = ((c << 16) | d) >>> 0;
  const at = hits[0];
  const before = [8, 12].map((o) => buf.readUInt32LE(at + o));
  const fmt = (hi, lo) => `${hi >>> 16}.${hi & 0xffff}.${lo >>> 16}.${lo & 0xffff}`;
  // dwFileVersionMS/LS at +8/+12, dwProductVersionMS/LS at +16/+20.
  buf.writeUInt32LE(ms, at + 8);
  buf.writeUInt32LE(ls, at + 12);
  buf.writeUInt32LE(ms, at + 16);
  buf.writeUInt32LE(ls, at + 20);
  return fmt(before[0], before[1]);
}

const buf = readFileSync(exe);
const was = patchFixedFileInfo(buf, numeric);
writeFileSync(exe, buf);
console.log(`set-exe-version: ${exe}: numeric FileVersion/ProductVersion ${was} -> ${numeric} (app version ${version})`);
