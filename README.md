# SeekerMail Desktop

> **A local-first desktop email client** where every account is an **AI "digital employee"** — with a role, an authorization level, and a decision scope — powered by a local semantic vector index, the **GTE (Global Tactical Engine)**.

> **Status:** `Public preview` · version `0.1.0`. This repository and its installers are an **early-access preview**, published openly so you can try SeekerMail and send feedback. It is **not** the commercial v1.0 release: builds are provided **as-is, with no SLA and no warranty**, and breaking changes are expected. Substantial backend and frontend already exist (IMAP sync, the local index, the AI pipeline, BYO AI providers, search, export); the mail send/receive pipeline was recently wired end-to-end.

> Here, **"public" means the source and preview builds are open to look at and try — not that the product is finished or generally available.** Maturity is tracked separately by the version number (see [Version Roadmap](#version-roadmap)).

> ⚠️ **Build note:** The default compile runs **offline stand-ins** (no real mail). Real IMAP/SMTP/OAuth need `--features live-net` — the `pnpm tauri:dev` / `pnpm tauri:build` scripts pass it for you. A bare `cargo build` without the flag will **refuse to send** (rather than silently pretend).

---

## Tech Stack

| Layer | Choice |
| --- | --- |
| Shell | Tauri 2.x |
| Backend | Rust (`sqlx` + SQLite, `async-imap`, `lettre`, `reqwest`) |
| Frontend | React 18 + TypeScript (Vite, TanStack Query, Zustand, Tailwind) |
| Local index | **GTE** — local semantic vector index (brute-force cosine over a local snapshot today; a LanceDB-class ANN store is the scaling target) |
| Local models | optional: ONNX embeddings (`--features local-embed`), GGUF local LLM (`--features local-llm`) |
| Form | Desktop app, local-first, data stays on device |

---

## Repository Layout

```
seekermail-desktop/        ← this repo: code only, public on GitHub
├── CLAUDE.md              ← AI context for coding
├── README.md              ← this file
├── package.json           ← frontend scripts & deps
├── src/                   ← React frontend
├── src-tauri/             ← Rust backend + Tauri config + migrations
│   ├── src/               ← Rust modules (imap / storage / ai / search / …)
│   └── migrations/        ← forward-only SQLite migrations
├── packages/shared/       ← generated Rust→TS bindings
└── xtask/                 ← bench + safety-gate tooling
```

> 📁 Product docs, specs, design drafts, and release/audit reports are **not** in this repo — they live in the separate private knowledge base `seekermail-desktop-2026/` (never pushed to GitHub).

---

## Build & Run

**Prerequisites:** Node `>=20 <23` · pnpm `9` · Rust (stable toolchain) · Tauri OS prerequisites.

```bash
pnpm install              # install frontend deps
pnpm setup:model          # fetch the local embedding model (optional)

pnpm tauri:dev            # desktop app (with --features live-net, real mail)
pnpm dev                  # frontend only (Vite, mocked IPC)

pnpm check                # tsc + eslint + prettier
pnpm test                 # frontend unit tests (Vitest)
pnpm gen:types            # regenerate the Rust→TS type bindings

pnpm tauri:build          # production build (with --features live-net)
```

> Backend tests on a Mac: `cargo test --manifest-path src-tauri/Cargo.toml`.
> To type-check the real network path: `cargo check --features live-net`.

Engineering specs (in the knowledge base):

- `../seekermail-desktop-2026/docs/dev/01_DATABASE_SCHEMA.md`
- `../seekermail-desktop-2026/docs/dev/02_IPC_COMMAND_CONTRACTS.md`
- `../seekermail-desktop-2026/docs/dev/03_RUST_MODULE_INTERFACES.md`
- `../seekermail-desktop-2026/docs/dev/05_PACKAGING_AND_NOTARIZATION.md`

---

## Preview Builds

Early-access installers are shared for **evaluation only** — they are previews, not production-signed releases. Once published, official preview builds will appear on the GitHub **Releases** page, each matched to a version tag. These are early-access previews (no SLA, no warranty), provided for evaluation; breaking changes are expected. Your rights in the source and the builds are governed by the [License](#license) - AGPL-3.0, with a commercial option for proprietary use.

---

## Language Rules

- Code, comments, identifiers, and UI copy are **English only** (except the i18n feature).
- Communication with the team/user is in **Chinese**.

---

## Version Roadmap

`v0.1 Preview → v0.4 Beta → v0.5 → v0.6 → v0.7 RC → v1.0 GA`

Two different things are tracked here, and they should not be confused:

- **Visibility** — the source and preview builds are **public now**, at `0.1.0`.
- **Maturity** — the product is **pre-GA**. `v1.0` is the **first commercial, generally-available release** (paid, with an SLA). Everything before it, including this preview, is early-access: **no charge, no SLA, and breaking changes are expected**.

So "v0.1 is public" and "v0.1 is not a finished release" are both true at once — open to try, not yet done.

Full plan in the knowledge base: `../seekermail-desktop-2026/docs/planning/01_VERSION_ROADMAP.md`.

---

## License

SeekerMail Desktop is **open source under the [GNU AGPL-3.0](LICENSE)**, run as **open core with dual licensing**:

- **Open-source edition (this repo).** Licensed under the **GNU Affero General Public License v3.0 (AGPL-3.0-only)**. You may use, study, modify, and redistribute it under the AGPL. Note AGPL section 13: if you run a modified version as a network service, you must offer its complete source to that service's users.
- **Commercial license.** If you cannot or do not want to comply with the AGPL (for example, to embed SeekerMail in a closed-source product or ship a proprietary derivative), a separate commercial license is available. See [`LICENSING.md`](LICENSING.md) or contact **support@vivbase.com**.
- **Pro / Enterprise features** are a separate, proprietary add-on and are **not** part of the AGPL grant in this repository.

Contributions are accepted under a Contributor License Agreement (see [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`CLA.md`](CLA.md)), which is what lets the project offer both the open-source and commercial licenses. Third-party components are credited in [`NOTICE`](NOTICE). Copyright (C) 2026 vivbase.
