# Contributing

## Workstreams and branches

Each workstream has one long-lived feature branch, owned by one teammate. Open PRs
from that branch (or short-lived branches cut from it) **into `main`**.

| Workstream | Branch | Label | Owns (may edit freely) |
|---|---|---|---|
| Device & protocol | `feat/device` | `device` | `src-tauri/src/device/**` |
| Flasher & firmware service | `feat/flasher` | `flasher` | `src-tauri/src/flasher/**` |
| Config engine | `feat/config` | `config` | `src-tauri/src/config/**`, `src-tauri/tests/fixtures/**` |
| Wizard UI & i18n | `feat/ui` | `ui` | `src/**` except `src/api/commands.ts` and `src/types/model.ts`, `tests/**` |
| Packaging, drivers, CI | `feat/packaging` | `packaging` | `src-tauri/src/system/**`, `src-tauri/tauri.conf.json`, `src-tauri/icons/**`, `src-tauri/windows/**`, `.github/**` |

### Shared files (label `shared`)

`src-tauri/src/lib.rs`, `src-tauri/src/model.rs`, `src-tauri/src/error.rs`,
`src/types/model.ts`, `src/api/commands.ts`, `schemas/**`, `docs/ARCHITECTURE.md`.

- Change them in a **small, separate PR** labelled `shared`, updating every mirror
  (Rust type ⇄ TS type ⇄ JSON schema ⇄ ARCHITECTURE.md) in the same PR.
- All commands are already registered in `lib.rs`; you should not need to touch it
  unless you add a new command (that is a `shared` change).
- Adding a dependency to `Cargo.toml` / `package.json`: separate commit, explain why;
  rebase on `main` right before merge to avoid lockfile conflicts.

## Workflow

1. `git checkout -b feat/<workstream> origin/main` (first time) and keep it rebased on `main`.
2. Small PRs, one issue each (`Closes #N`), title prefixed with the workstream: `device: MSP v2 framing`.
3. CI must be green: `npm test`, `npm run build`, `cargo test`, Windows bundle.
4. Squash-merge into `main`.

## Code rules

- Rust: `cargo fmt`, no `unwrap()` in command paths, return `AppError`.
- Commands keep the signatures in `docs/ARCHITECTURE.md` §3.
- UI never imports `@tauri-apps/api` outside `src/api/`.
- Every user-visible string goes through vue-i18n with **both** `he` and `en` keys.
- Hardware-free tests first: config golden tests with recorded CLI transcripts; UI tests
  with the mock backend.
- Safety: any feature that spins motors must require an explicit "props removed" confirmation.

## License

By contributing you agree your contributions are licensed under GPL-3.0-or-later.
