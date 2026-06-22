# SeekerMail Desktop

> **A local-first desktop email client** where every account is an **AI "digital employee"** — with a role, an authorization level, and a decision scope — powered by a local semantic vector index, the **GTE (Global Tactical Engine)**.

> **Status:** `In development` — `package.json` version `0.1.0`; **not yet publicly released.** Substantial backend + frontend already exist (IMAP sync, the local index, the AI pipeline, BYO AI providers, search, export); the mail send/receive pipeline was just wired end-to-end.

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

> 📁 Product docs, specs, and design drafts are **not** in this repo — they live in the separate private knowledge base `seekermail-desktop-2026/` (never pushed to GitHub).

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

## Language Rules

- Code, comments, identifiers, and UI copy are **English only** (except the i18n feature).
- Communication with the team/user is in **Chinese**.

---

## Version Roadmap

`v0.1 Internal → v0.4 Beta → v0.5 → v0.6 → v0.7 RC → v1.0 GA`

v1.0 is the **first public + commercial release**; v0.1–v0.7 are internal/closed validation (not public, not charged, no SLA). The project is at an early implementation stage of that roadmap (version `0.1.0`); the `v1.0.0_*` files under `docs/releases/` are **forward-looking drafts / gate-evidence templates**, not a shipped release.

Full plan in the knowledge base: `../seekermail-desktop-2026/docs/planning/01_VERSION_ROADMAP.md`.

---

## License

TBD — commercial product; the `LICENSE` will be finalized and added before public release.
