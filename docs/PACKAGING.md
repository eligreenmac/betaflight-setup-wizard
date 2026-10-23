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
  `action = INSTALL_DFU_DRIVER`.
- **Install only when needed** (`system_install_dfu_driver`): for each distinct DFU
  VID:PID that needs it, start `<resources>/drivers/bfsw-wdi-simple.exe` with
  `ShellExecuteExW("runas")` → one UAC prompt per device type (normally one). The main
  app never runs elevated. Afterwards the status is re-read; the command fails with
  `DRIVER` if the device is still not on WinUSB, the user declined UAC, or libwdi
  returned an error (message includes the libwdi error text).
- **Helper** = libwdi's unmodified `wdi-simple` example, v1.5.1 (commit pinned), built
  by `.github/actions/build-wdi-simple` on `windows-latest` with MSVC exactly like
  libwdi's own VS2022 CI (WDK 8.0 redistributable for the WinUSB co-installers),
  WinUSB-only (libusb-win32/libusbK payloads disabled). The result is cached by commit.
  Command line: `--vid 0x0483 --pid 0xDF11 --type 0 --name "... (Betaflight Setup
  Wizard)" --inf bfsw_0483_df11.inf --dest %TEMP%\bfsw-winusb\0483_df11 --silent
  --timeout 120000`.
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
    case "INSTALL_DFU_DRIVER": /* button → */ await api.system.installDfuDriver(); break;
    case "VENDOR_DOWNLOAD":    /* show */ s.guidance[d.guidanceIndex!]; break; // open downloadUrl in the system browser
    case "REPLUG":             /* "unplug and replug / check Device Manager" */ break;
  }
}
```

- Use `guidance[i].id` for i18n keys (e.g. `drivers.guidance.CP210X`); `vendor`,
  `chips`, `downloadUrl` are data, not translations. `notes` are English diagnostics.
- `installDfuDriver()` may take a while (UAC + driver install, up to minutes); show a
  spinner and handle `{code: "DRIVER"}` (declined UAC or failure) with a retry.
- Opening `downloadUrl` in the browser needs an opener (e.g. `tauri-plugin-opener`),
  which is not wired yet — UI/shared follow-up.
- Non-Windows: `dfuWinusb = vcp = true`, `devices = []`.

## 3. CI (issue #25)

`.github/workflows/build.yml`: windows-latest (helper build, `npm ci/test/build`,
`cargo test`, `tauri build` NSIS+MSI, installer check, artifacts) and ubuntu
(`cargo fmt/check/test`). All actions run on Node 24.

## 4. Release pipeline (issue #28)

`.github/workflows/release.yml`

1. Bump `version` in `src-tauri/tauri.conf.json` (and `Cargo.toml`/`package.json`)
   on `main`.
2. A maintainer pushes tag `vX.Y.Z` (must equal that version; the workflow fails
   otherwise).
3. windows-latest builds the helper, runs tests, `tauri build` (NSIS + MSI), checks the
   helper is in the installer, packs `libwdi-<ref>-source.zip`, writes
   `SHA256SUMS.txt` (spaces in file names become `_` so names match the assets).
4. A second job (only for tags, `contents: write`) creates a **draft** release with
   those files, or re-uploads to the draft if it already exists. It refuses to touch
   an already published release. **Publishing is manual.**

PRs that change `release.yml` or the helper action, and manual runs, execute only the
build job (dry run with an artifact).

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
