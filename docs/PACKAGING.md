# Packaging, drivers and CI (workstream `packaging`)

Owner: `feat/packaging`. Code: `src-tauri/src/system/**`, `src-tauri/drivers/`,
`src-tauri/tauri.conf.json`, `.github/**`.

## 1. WinUSB for DFU (issue #26)

Flashing over USB DFU needs the **WinUSB** driver bound to the MCU ROM bootloader.
Windows binds nothing to these IDs by itself:

| MCU | DFU VID:PID |
|---|---|
| STM32 (MVP) | `0483:DF11` |
| AT32F435 (MVP) | `2E3C:DF11` |
| APM32 | `314B:0106` |
| GD32 | `28E9:0189` |
| X32 | `3797:DF11` |

(Source: Betaflight App `src/js/protocols/devices.ts`; table in
`src-tauri/src/system/usb_drivers.rs`.)

### Chosen approach: libwdi `wdi-simple`, built in CI, started elevated on demand

- **Detect first, unelevated** (`system_driver_status`): SetupAPI lists present
  `USB\VID_xxxx&PID_yyyy` device nodes with their bound service (`SPDRP_SERVICE`) and
  Device Manager problem code. A DFU device is OK only if bound to `WinUSB` with no
  problem. Another driver (e.g. ST's DfuSe `STTub30`, libusb0) or none →
  `action = INSTALL_DFU_DRIVER`; with another driver bound, `replacesDriver = true`
  (WinUSB will replace it, so the UI warns first).
- **Install only when needed** (`system_install_dfu_driver`): the app relaunches
  **its own exe** with `ShellExecuteExW("runas")` and
  `--bfsw-elevated-winusb-install <vid>:<pid>:<hex name>...` → **one UAC prompt**,
  naming the app, for all DFU VID:PIDs that need it. `main()` hands that process to
  `system::elevated_install::dispatch` before any Tauri/WebView code runs. The main
  app process never runs elevated. The elevated process
  (`src-tauri/src/system/elevated_install.rs`):
  1. atomically creates (`CreateDirectoryW`, fails if it exists) a fresh
     `%ProgramData%\BFSW-WinUSB-<run id>` (`%ProgramData%` from
     `SHGetKnownFolderPath`, not the environment) with the protected DACL
     `O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)` (SYSTEM + Administrators only).
     It then opens the folder without following reparse points and without delete
     sharing, so it can't be renamed or replaced while in use. It refuses to continue
     if the folder is a reparse point, its owner/DACL read back is anything else,
     or it is not empty;
  2. copies the helper from `<exe dir>\drivers\bfsw-wdi-simple.exe` (the Tauri
     resource dir; never a path taken from the command line) into it and runs it from
     there, once per VID:PID with `--dest <dir>\<vid>_<pid>` (stops at the first
     failure). Device names are hex-encoded on the command line and re-validated
     (no control characters, ≤128 chars);
  3. deletes the folder on every exit path (only if it created and verified it).
  Driver files are therefore never extracted to the user-writable `%TEMP%`, where
  unelevated code could swap the `.inf/.cat`/co-installer between extraction and
  install. Afterwards the status is re-read. The command fails with `DRIVER` if a device
  is still not on WinUSB, the user declined UAC, the folder checks failed, or libwdi
  returned an error (the message includes the libwdi error text). Exit codes:
  0, libwdi's negative codes, or 1001–1009 (see `exit_message`).
  The UAC prompt shows the app's name from its version resource. It shows a verified
  publisher only once the exe is Authenticode-signed; unsigned builds show
  "Unknown publisher". Residual risk, unchanged: with a per-user NSIS install the exe
  and helper live in a user-writable folder, so the UAC prompt is only as trustworthy
  as that folder.
- **Helper** = libwdi's unmodified `wdi-simple` example, v1.5.1 (commit pinned), built
  by `.github/actions/build-wdi-simple` on `windows-latest` with MSVC exactly like
  libwdi's own VS2022 CI (WDK 8.0 redistributable for the WinUSB co-installers),
  WinUSB-only (libusb-win32/libusbK payloads disabled) and x86/x64-only (ARM64 installer
  disabled: hosted runners lack the v143 ARM64 tools and the app ships x64 only). The
  result is cached by commit. The WDK redist MSI is pinned by SHA256
  (`wdk-redist-sha256` input,
  `29314207814ce9d5d73695f7e9239539cf37c79e750b9d5ea5a5ef5487a583d6`, Authenticode
  "Microsoft Corporation" 2012); the build fails hard on a mismatch. Nothing is
  downloaded at runtime: the co-installers are embedded in the helper at build time.
  Command line: `--vid 0x0483 --pid 0xDF11 --type 0 --name "... (Betaflight Setup
  Wizard)" --inf bfsw_0483_df11.inf --dest %ProgramData%\BFSW-WinUSB-<id>\0483_df11
  --silent --timeout 120000`.
- The exe is **not committed**; `src-tauri/drivers/*` is bundled as Tauri resources,
  and CI fails if the NSIS installer does not contain it.

Why not the alternatives:
- *Pre-signed driver package (INF + CAT)*: on x64 Windows a catalog must be signed by
  a certificate chained to a trusted root; an INF referencing inbox `winusb.inf` still
  needs a signed catalog. That needs an EV certificate + Microsoft attestation signing
  (not available yet; revisit with code signing in phase 3).
- *libwdi as a Rust FFI dependency inside the app*: would make the main exe a combined
  LGPL work and require relinking provisions; a separate helper exe is simpler and
  keeps the app unelevated.
- *Bundling Zadig / ImpulseRC Driver Fixer*: UI tools, not scriptable/silent; the
  latter is closed source.

What libwdi does on the user's PC: generates an `.inf/.cat` for the device, creates a
self-signed certificate, adds it to the machine's Trusted Root/Trusted Publisher stores
(private key discarded), and installs the driver package (same as Zadig).

### LGPL-3.0 obligations for the helper

`wdi-simple` and libwdi are LGPL-3.0-or-later. We distribute the helper as a separate
program, so:

1. Ship the license texts and a notice with it → `src-tauri/drivers/LIBWDI-NOTICE.txt`,
   `COPYING.LGPL-3.0.txt`, `COPYING.GPL-3.0.txt` (bundled next to the exe).
2. Provide the Corresponding Source → upstream tag + **`libwdi-v1.5.1-source.zip`
   attached to every GitHub release** by the release workflow (issue #28), plus our
   build recipe (the composite action) in this repo.
3. Allow replacement → the app only executes the helper by path; users can rebuild
   and swap it.
4. When bumping libwdi: update the `libwdi-ref`/`libwdi-commit` inputs, the notice
   file, and the release workflow's source archive in the same PR.

## 2. USB-UART bridges: CP210x / CH34x (guidance only)

Their vendor drivers are not ours to redistribute. `system_driver_status` returns:

- `devices[]` entries with `kind = USB_UART_BRIDGE`; when no driver is bound (or the
  device has a problem code) `action = VENDOR_DOWNLOAD` and `guidanceIndex` points
  into `guidance[]`;
- `guidance[]` (always filled, also for the "nothing detected" help screen):

| id | Vendor | IDs | Official page |
|---|---|---|---|
| `CP210X` | Silicon Labs | 10C4:EA60, EA61, EA62, EA70, EA71 | https://www.silabs.com/software-and-tools/usb-to-uart-bridge-vcp-drivers |
| `CH34X` | WCH | 1A86:7522, 7523, 7584, 5523 | https://www.wch-ic.com/downloads/CH341SER_EXE.html |
| `CH343` | WCH | 1A86:55D3, 55D4 | https://www.wch-ic.com/downloads/CH343SER_EXE.html |

### For the UI (`src/api` already exposes these)

```ts
const s = await api.system.driverStatus();
for (const d of s.devices.filter(d => !d.driverOk)) {
  switch (d.action) {
    case "INSTALL_DFU_DRIVER": // if d.replacesDriver: warn "replaces <d.service>" + confirm first
      /* button → */ await api.system.installDfuDriver(); break;
    case "VENDOR_DOWNLOAD":    /* show */ s.guidance[d.guidanceIndex!]; break; // open downloadUrl in the system browser
    case "REPLUG":             /* "unplug and replug / check Device Manager" */ break;
  }
}
```

- Use `guidance[i].id` for i18n keys (e.g. `drivers.guidance.CP210X`); `vendor`,
  `chips`, `downloadUrl` are data, not translations. `notes` are English diagnostics.
- `replacesDriver = true` (DFU device currently bound to another driver, e.g.
  `STTub30` from ST DfuSe/older CubeProgrammer, or libusb0): show a warning naming
  `service` and require explicit confirmation before `installDfuDriver()`; tools that
  depend on that driver stop working with the board until it is switched back in
  Device Manager.
- `installDfuDriver()` may take a while (UAC + driver install, up to minutes); show a
  spinner and handle `{code: "DRIVER"}` (declined UAC or failure) with a retry.
- Open `downloadUrl` with `openExternal()` from `src/api/opener.ts`. Its allow-list
  (and `opener:allow-open-url` in `src-tauri/capabilities/default.json`) does not yet
  include `https://www.silabs.com/*` and `https://www.wch-ic.com/*`; adding them is a
  small `shared` follow-up (both lists must change together).
- Non-Windows: `dfuWinusb = vcp = true`, `devices = []`.

## 3. CI (issue #25)

`.github/workflows/build.yml`: windows-latest (helper build, `npm ci/test/build`,
`cargo test`, `tauri build` NSIS+MSI, installer check, artifacts) and ubuntu
(`cargo fmt/check/test`). All actions run on Node 24.

## 4. Release pipeline (issue #28)

`.github/workflows/release.yml`

1. Bump `version` in `src-tauri/tauri.conf.json`, `package.json` + `package-lock.json`
   (`npm version <v> --no-git-tag-version`) and `src-tauri/Cargo.toml` (+ `Cargo.lock`)
   on `main`. `.github/scripts/check-release-version.mjs` fails the release if they
   differ.
2. A maintainer pushes tag `v<version>` (must equal that version; the workflow fails
   otherwise).
3. windows-latest builds the helper, runs tests, `tauri build` (NSIS + MSI), checks the
   helper is in the installer, packs `libwdi-<ref>-source.zip`, writes
   `SHA256SUMS.txt` (spaces in file names become `_` so names match the assets).
4. A second job (only for tags, `contents: write`) creates a **draft** release with
   those files, or re-uploads to the draft if it already exists. It refuses to touch
   an already published release. **Publishing is manual.**

PRs that change `release.yml` or the helper action, and manual runs, execute only the
build job (dry run with an artifact).

### Versions (MSI rules)

Windows Installer versions are `major.minor.build[.rev]` (major/minor <= 255, build
<= 65535). Tauri's WiX bundler maps SemVer `X.Y.Z-N` to `X.Y.Z.N` and rejects any
non-numeric or > 65535 pre-release (e.g. `-beta.1`) or `+build` metadata. So:

- Release: `X.Y.Z` (tag `vX.Y.Z`).
- Preview: `X.Y.Z-N` (tag `vX.Y.Z-N`), e.g. the first preview `0.1.0-1` → MSI
  `0.1.0.1`. The draft is marked as a GitHub **pre-release**. The workflow checks the
  built MSI's `ProductVersion`.
- Windows Installer ignores the 4th field when it compares versions, and Tauri's MSI
  uses `MajorUpgrade AllowSameVersionUpgrades="yes"`, so `0.1.0-1` → `0.1.0-2` →
  `0.1.0` install over each other as upgrades. In SemVer `0.1.0-1` < `0.1.0`, so the
  final release still sorts after its previews.

### Windows file versions (NSIS installer and app exe)

The numeric `VERSIONINFO` (fixed `FileVersion` and `ProductVersion`, which Explorer and
inventory tools show) follows the same rule as the MSI. `X.Y.Z-N` → `X.Y.Z.N`;
`X.Y.Z` → `X.Y.Z.0`, because a VERSIONINFO always has 4 fields. The string
`FileVersion`/`ProductVersion` and the NSIS display version stay `X.Y.Z-N`.
`src-tauri/windows/version.mjs` holds the rule, and
`check-release-version.mjs` outputs it as `windows_version`.

Tauri itself only puts semver `+build` metadata in the 4th field (which MSI forbids),
so `0.1.0-1` came out as `0.1.0.0` (v0.1.0-1's setup.exe and app exe). Two pieces fix that:

- **NSIS installer**: `bundle.windows.nsis.template` = `src-tauri/windows/installer.nsi`.
  This is Tauri's own template for the locked `@tauri-apps/cli` version, where only
  `VIProductVersion`/`VIFileVersion` are computed from `${VERSION}` with
  `!searchparse`.
  - `installerHooks` can't do it: hooks are `!include`d before `VERSIONWITHBUILD` is
    defined and before `VIProductVersion` is set, and NSIS rejects redefining either.
  - The file is generated, not hand-edited: after updating `@tauri-apps/cli`, run
    `node src-tauri/windows/nsis-template.mjs --write`.
  - CI (`--check` in `build.yml`) fails if the file differs from that version's upstream
    template plus the change. So a CLI bump can't silently pair a stale template with a
    newer bundler.
- **App exe**: `build.beforeBundleCommand` runs `src-tauri/windows/set-exe-version.mjs`.
  It runs after `cargo build` and before bundling and signing. It overwrites the 16
  version bytes of the exe's `VS_FIXEDFILEINFO` in place: same size, idempotent, and it
  honours `tauri build --config` version overrides. Both bundles then ship the patched
  exe. On non-Windows builds it does nothing.

CI (`build.yml` and `release.yml`, Windows) runs
`.github/scripts/check-windows-versions.ps1`. It reads both numeric fields of the built
`*-setup.exe` and `target/release/betaflight-setup-wizard.exe` and fails if they differ
from `windows_version`, next to the MSI `ProductVersion` check.

### Not enabled yet (later phases)

- **Code signing** (phase 3). Plan: Azure Trusted Signing (cheapest, no HSM token)
  or an OV/EV certificate. Hooks are marked `TODO(code signing)` in `release.yml`:
  sign `bfsw-wdi-simple.exe` right after it is built (it is what the UAC prompt
  shows), then let Tauri sign the app/NSIS/MSI via `bundle.windows.signCommand`
  with secrets. Until then SmartScreen and UAC show "unknown publisher".
- **Auto-update**: `bundle.createUpdaterArtifacts` is off and no updater plugin is
  configured; enabling it requires generating the Tauri updater key pair, storing
  `TAURI_SIGNING_PRIVATE_KEY` (+ password) as repo secrets and keeping an offline
  backup (losing it breaks updates for every installed copy).
