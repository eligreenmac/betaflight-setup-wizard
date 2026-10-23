# Betaflight Setup Wizard

A Windows desktop app that takes **any** Betaflight-capable flight controller from
"just plugged in" to "configured" — no website needed:

1. Auto-detects the flight controller (USB VID/PID, MSP board info, DFU mode).
2. Flashes Betaflight (cloud build once, then cached for offline use).
3. Asks simple questions — what is connected, and to which pad/UART
   (receiver, VTX / digital air unit, GPS, ESC protocol, switches…).
4. Generates and applies the right settings, with backup, verification and rollback.

UI in **Hebrew (RTL)** and **English**. Built with **Tauri 2** (Rust core) and
**Vue 3 + Vite + TypeScript**.

> Status: early development (Phase 0 – foundation). Not usable yet.
> This is an independent project, not affiliated with the Betaflight team.

## Docs
- [Product plan](docs/PLAN.md)
- [Architecture & module contracts](docs/ARCHITECTURE.md)
- [Contributing](CONTRIBUTING.md)
- [DroneModel JSON schema](schemas/drone-model.schema.json)

## Development

Prerequisites: Node 22+, Rust stable ≥ 1.90, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
(WebView2 on Windows; webkit2gtk-4.1 on Linux).

```bash
npm install
npm test                 # Vitest (UI)
npm run tauri dev        # run the desktop app
npm run tauri build      # build installers (NSIS/MSI on Windows)
cd src-tauri && cargo test
```

UI-only development without hardware: the UI talks to Rust only through
`src/api/commands.ts`; tests use `setBackend(createMockBackend())` from `src/api/mock.ts`.

## Layout

```
src/                 Vue UI (wizard steps, i18n, typed API wrapper)
src-tauri/src/
  device/            USB detection, serial, MSP, CLI
  flasher/           Build API client, firmware cache, DFU flashing
  config/            DroneModel -> CLI, backup/apply/verify/rollback
  system/            Windows drivers (WinUSB/libwdi), app info
  model.rs           shared types (mirrors src/types/model.ts)
schemas/             DroneModel JSON schema
docs/                plan & architecture
```

## License

GPL-3.0-or-later — see [LICENSE](LICENSE). Portions are derived from or informed by the
[Betaflight App](https://github.com/betaflight/betaflight-configurator) (GPL-3.0).
