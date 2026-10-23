# drivers/ — bundled WinUSB helper (packaging workstream)

Everything in this folder is shipped inside the installer as Tauri resources
(`tauri.conf.json` → `bundle.resources: ["drivers/*"]`) and ends up in
`<install dir>/drivers/`.

| File | Committed? | What |
|---|---|---|
| `bfsw-wdi-simple.exe` | **no** (built in CI, git-ignored) | libwdi's unmodified `wdi-simple` example, built from a pinned tag by `.github/actions/build-wdi-simple`. Started elevated by `system_install_dfu_driver` to bind WinUSB to STM32/AT32 DFU devices. |
| `LIBWDI-NOTICE.txt` | yes | Attribution, license and source-offer for libwdi (LGPL-3.0-or-later). |
| `COPYING.LGPL-3.0.txt`, `COPYING.GPL-3.0.txt` | yes | License texts that must accompany the helper. |

Local Windows dev: the app works without the exe (the install command returns a
`DRIVER` error "helper not bundled"). To test driver install locally, build it
the same way as CI (see `docs/PACKAGING.md`) and drop it here.
