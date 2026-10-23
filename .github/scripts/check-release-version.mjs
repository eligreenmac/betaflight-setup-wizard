// SPDX-License-Identifier: GPL-3.0-or-later
// Release version gate (used by .github/workflows/release.yml).
//
// 1. All version sources agree: src-tauri/tauri.conf.json (source of truth),
//    package.json, package-lock.json, src-tauri/Cargo.toml.
// 2. The version is something Tauri's WiX (MSI) bundler accepts:
//    MAJOR.MINOR.PATCH[-N] where N is a single numeric pre-release identifier
//    <= 65535 (it becomes the 4th MSI field: 0.1.0-1 -> 0.1.0.1), MAJOR/MINOR <= 255,
//    PATCH <= 65535, no +build metadata. Example preview: 0.1.0-1 (tag v0.1.0-1).
// 3. On a tag build, the tag is exactly "v" + version.
//
// Usage: node check-release-version.mjs [repo-root] [tag]   (tag defaults to
// $GITHUB_REF_NAME when $GITHUB_REF is a tag). Writes version= and prerelease= to
// $GITHUB_OUTPUT when set. Exit code 1 with a message on any problem.
import { readFileSync, appendFileSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2] ?? ".";
const tag =
  process.argv[3] ??
  (process.env.GITHUB_REF?.startsWith("refs/tags/") ? process.env.GITHUB_REF_NAME : undefined);

const read = (p) => readFileSync(join(root, p), "utf8");
const errors = [];

const version = JSON.parse(read("src-tauri/tauri.conf.json")).version;
const sources = {
  "package.json": JSON.parse(read("package.json")).version,
  "package-lock.json": JSON.parse(read("package-lock.json")).version,
  'package-lock.json packages[""]': JSON.parse(read("package-lock.json")).packages?.[""]?.version,
  "src-tauri/Cargo.toml": /^\[package\][^[]*?^version\s*=\s*"([^"]+)"/ms.exec(read("src-tauri/Cargo.toml"))?.[1],
};
for (const [file, v] of Object.entries(sources)) {
  if (v !== version) errors.push(`${file} has version ${v}, tauri.conf.json has ${version}`);
}

export function msiProblem(v) {
  const m = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(0|[1-9]\d*))?$/.exec(v ?? "");
  if (!m) {
    return `version "${v}" must be MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-N with numeric N (MSI/WiX); no +build metadata`;
  }
  const [maj, min, pat, pre] = m.slice(1).map((x) => (x === undefined ? undefined : Number(x)));
  if (maj > 255 || min > 255) return `MSI limits MAJOR and MINOR to 255 (got ${v})`;
  if (pat > 65535) return `MSI limits PATCH to 65535 (got ${v})`;
  if (pre !== undefined && pre > 65535) return `MSI limits the numeric pre-release to 65535 (got ${v})`;
  return null;
}

const msi = msiProblem(version);
if (msi) errors.push(msi);
if (tag !== undefined && tag !== `v${version}`) {
  errors.push(`tag ${tag} does not match tauri.conf.json version ${version} (expected v${version})`);
}

if (errors.length) {
  for (const e of errors) console.log(`::error::${e}`);
  process.exit(1);
}
const prerelease = version.includes("-");
console.log(`version ${version} OK${tag ? ` (tag ${tag})` : ""}; prerelease=${prerelease}`);
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\nprerelease=${prerelease}\n`);
}
