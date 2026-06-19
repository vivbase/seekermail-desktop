# SeekerMail Desktop — Code Repo Context for Claude

This is the **code repository** for SeekerMail. It is public and pushed to GitHub. It contains **only source code** — no product specs, no design drafts.

## Two-Repo Model (read this first)

| Folder | Role | On GitHub? |
|---|---|---|
| `seekermail-desktop/` (here) | Code | **Yes** (public) |
| `seekermail-desktop-2026/` (sibling) | Knowledge base: docs, design, prototypes | **No** (private/local) |

- **Specs and design are NOT in this repo.** They live in the sibling `seekermail-desktop-2026/` knowledge base. When you need product/IA/feature/engineering detail, read from there — do **not** copy those docs into this repo.
- **Never commit** product documents, design files, or anything from the knowledge base into this repo.
- The big project-context file (product definition, design system, i18n spec, doc governance) is `../seekermail-desktop-2026/CLAUDE.md`. This file is the lighter, code-focused companion.

## What This Project Is

A **local-first desktop email client** built on **Tauri 2.x + Rust + React 18 (TypeScript)**, with a local semantic vector index (**GTE** — today a brute-force cosine over a JSON snapshot in `src-tauri/src/vector/`; a LanceDB-class ANN store is the scaling target, not the current implementation). Each email account is an autonomous **AI agent ("digital employee")** with a role and an authorization level (`Full Auto` / `Semi-Auto` / `Manual Only`). Treat every output as production-quality — no placeholder, "test", "demo", "TODO", or lorem ipsum copy.

## Stack & Where Code Goes

| Path | Holds |
|---|---|
| `src-tauri/` | Rust backend, Tauri config, IPC commands, migrations |
| `src/` | React 18 + TypeScript frontend |

Product code starts at **v0.1** (engineering skeleton + single-account fetch). Until then these directories are scaffolding.

## Key Specs (in the sibling knowledge base)

- Architecture / DB: `../seekermail-desktop-2026/docs/dev/01_DATABASE_SCHEMA.md`
- IPC contracts: `../seekermail-desktop-2026/docs/dev/02_IPC_COMMAND_CONTRACTS.md`
- Rust module interfaces: `../seekermail-desktop-2026/docs/dev/03_RUST_MODULE_INTERFACES.md`
- Performance targets: `../seekermail-desktop-2026/docs/dev/04_PERFORMANCE_BENCHMARKS.md`
- Packaging/notarization: `../seekermail-desktop-2026/docs/dev/05_PACKAGING_AND_NOTARIZATION.md`
- BYO-AI integration: `../seekermail-desktop-2026/docs/dev/06_API_INTEGRATION_SPEC.md`
- PRD / IA / features: `../seekermail-desktop-2026/docs/product/` and `../seekermail-desktop-2026/docs/function list/`
- Design system & tokens: `../seekermail-desktop-2026/UI/Seeker Design System/`
- Version plan: `../seekermail-desktop-2026/docs/planning/`

## Language Rules (critical)

- **All code, comments, identifiers, and UI copy: English only** (the i18n feature is the sole exception; native-script language names are allowed there).
- **All communication with the user: Chinese (Simplified).**
- Never put Chinese text in any source, UI component, or config file.

## Conventions

- Follow the design-system tokens defined in `../seekermail-desktop-2026/UI/Seeker Design System/` — do not invent new colors/fonts.
- Use logical CSS properties (`padding-inline-start`, not `padding-left`) so RTL locales work.
- Keep secrets out of the repo (see `.gitignore`); commit a `.env.example`, never a real `.env`.
- Update `CHANGELOG.md` in the same change that ships a user-visible feature (the changelog currently lives in the knowledge base at `../seekermail-desktop-2026/CHANGELOG.md`; mirror or move it here once code lands).

## What To Never Do

- Never commit specs, design files, or knowledge-base content into this repo.
- Never use Chinese text in source/UI/config.
- Never add "test", "demo", "placeholder", "TODO", or "WIP" labels in shipped UI.
- Never commit secrets, `target/`, `node_modules/`, or build output.
