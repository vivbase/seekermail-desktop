# Contributing to SeekerMail

Day-one guide for the SeekerMail **code repo**: prerequisites, setup, the run/build/
test loop, layout, and conventions. This is the code-side mirror of the knowledge
base's `docs/dev/08_GETTING_STARTED.md`, calibrated to what **v0.4** actually
ships (accounts + sync + search + compose + settings/data management + the perf
and release tooling). When the two disagree about an existing command, this file
wins; for product/spec detail, read the knowledge base.

## 0. The two repos

| Folder                       | Role                                      | On GitHub          | You build from      |
| ---------------------------- | ----------------------------------------- | ------------------ | ------------------- |
| `seekermail-desktop/` (here) | Code (Tauri + Rust + React)               | Yes (public)       | **here**            |
| `seekermail-desktop-2026/`   | Knowledge base: specs, design, prototypes | No (private/local) | read-only reference |

Never commit specs into this repo; never commit code into the knowledge base.

## 1. Prerequisites

| Tool                             | Version                                       | Install                         |
| -------------------------------- | --------------------------------------------- | ------------------------------- | --- |
| Rust                             | stable (pinned by `rust-toolchain.toml`)      | `curl https://sh.rustup.rs -sSf | sh` |
| Node                             | 20 LTS (pinned by `.nvmrc`)                   | `nvm install`                   |
| pnpm                             | 9.x (pinned by `package.json#packageManager`) | `corepack enable`               |
| Tauri CLI                        | 2.x                                           | bundled via `pnpm tauri`        |
| Xcode Command Line Tools (macOS) | —                                             | `xcode-select --install`        |

macOS is the primary dev/release platform. The embedding model (bge-m3 ONNX) is
fetched by a script into `src-tauri/resources/` and is **not** committed.

## 2. First-time setup

```bash
nvm install            # reads .nvmrc
corepack enable        # pnpm at the pinned version
rustup show            # honours rust-toolchain.toml

pnpm install           # root + workspaces (incl. packages/shared)
pnpm run gen:types     # generate packages/shared/src/bindings.ts from Rust types
cp .env.example .env   # local, non-secret config (never committed)

# Optional now, required before semantic search (B3, v0.4):
pnpm run setup:model   # idempotent; ~2.2 GB; writes src-tauri/resources/ + model.lock.json
```

Runtime credentials (IMAP/SMTP/OAuth/AI keys) live in the OS Keychain — never in
`.env`, the DB, the frontend, or logs.

## 3. Run, build, test

```bash
pnpm tauri dev         # Tauri shell + Vite HMR
pnpm dev               # frontend only, mocked IPC (fast UI iteration)

pnpm check             # tsc --noEmit + eslint + prettier --check
bash scripts/check-boundaries.sh   # no bare hex; @tauri-apps/api only in src/ipc
pnpm test              # Vitest (frontend units + RTL)

cd src-tauri
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test             # unit + sqlx::test against temp SQLite
```

First `pnpm tauri dev` is slow (Rust compiles the dep tree once). If the frontend
fails to type-check against `@shared/bindings`, run `pnpm run gen:types`.

### Performance benchmarks (T055)

```bash
cargo xtask bench-seed --count 100000        # deterministic 100k corpus (seed=42)
cargo xtask bench --out bench-report.json --baseline bench-baseline.json
cargo xtask bench-gate --baseline bench-baseline.json --report bench-report.json
```

`bench` runs the M1–M8 harnesses (dev/04) and exits non-zero when any P0 metric
exceeds its threshold; `>baseline×1.10` prints an AMBER warning but passes.
`--smoke` shortens the M6/M7 windows for PR-level checks. The nightly gate runs
in `.github/workflows/bench.yml` on the Tier-A Apple Silicon runner.

### Release tooling (T056)

```bash
scripts/release_check.sh v0.4.0-beta   # manual pre-tag checklist (must pass)
git tag v0.4.0-beta && git push --tags # triggers .github/workflows/release.yml
```

The release workflow signs (hardened runtime + `src-tauri/entitlements.plist`),
notarizes, staples, Gatekeeper-verifies, and publishes the DMG + updater
`latest.json`. Secrets follow the dev/05 §1.1 names — never hardcode them.

## 4. Repository layout

```
src-tauri/                 # Rust backend
  src/
    main.rs                # shim → seekermail_lib::run()
    lib.rs                 # module tree + Tauri builder + generate_handler!
    commands/              # thin #[tauri::command] wrappers (one file per module)
    types.rs  error.rs     # specta DTOs + AppError/IpcError
    logging.rs  config.rs  # tracing init + app paths
    storage/  keychain/    # sqlx pool + migrations; OS Keychain vault
    account/  imap/  send/ # A-module: accounts, OAuth, sync scheduler, SMTP
    sanitize/              # B1 HTML sanitiser + B2 tracker policy
    embedding/  vector/    # B3 chunk→embed pipeline + derived vector index
    search/                # C1 FTS5 keyword + C2 semantic two-stage search
    exporter/              # H2 export: mbox / JSON Lines / zip writers
  examples/gen_bindings.rs # `pnpm gen:types` exporter (specta-export; example, not bundled)
  migrations/              # forward-only schema (001_init … 005_compose_drafts)
  entitlements.plist       # hardened-runtime minimal entitlements (release)
  deny.toml                # cargo-deny: license allow-list + advisories
  resources/               # bundled ONNX model (git-ignored) + model.lock.json
src/                       # React frontend (shell, routes, ipc, i18n, stores)
  routes/settings/         # H1: accounts/appearance/privacy/data(+sub-pages)/about
packages/shared/src/       # generated specta bindings (checked in)
xtask/                     # `cargo xtask` bench tooling (standalone crate)
bench-baseline.json        # perf baseline the nightly gate compares against
scripts/                   # setup-model.mjs, check-boundaries.sh, smoke.sh,
                           # smoke_v04.sh, release_check.sh
```

## 5. Type generation (specta)

Rust DTOs in `types.rs`/`error.rs` are the single source of truth. `pnpm gen:types`
runs the bindings example (`cargo run --features specta-export --example
gen_bindings`) and writes `packages/shared/src/bindings.ts` — **generated, do not
edit**. CI fails if the file drifts from the Rust types:

```bash
pnpm run gen:types
git diff --exit-code packages/shared/src/bindings.ts
```

> Note: the spec sketches this export in `build.rs`. A build script can't call into
> the crate's own type modules, so the skeleton uses a dedicated `gen_bindings`
> example instead — same drift-check contract.

## 6. Branches, commits, PRs

- Branch off `main`: `feat/<short>`, `fix/<short>`, `chore/<short>`.
- [Conventional Commits](https://www.conventionalcommits.org/); scope is the
  module/route (`feat(ipc): …`, `fix(pending): …`). Enforced by commitlint.
- Small, single-purpose PRs that reference the spec they advance. CI green required.
- Update `CHANGELOG.md` for user-visible changes in the same PR.

## 7. Gates

`pnpm check`, `cargo fmt --check`, `cargo clippy -D warnings`, `pnpm test`,
`cargo test`, the boundary greps, and the bindings drift check all run in CI
(`.github/workflows/ci.yml`, macOS-14). A lefthook pre-commit hook runs
prettier + eslint + rustfmt on staged files so CI rarely fails on style.

## 8. Definition of done

1. Behavior matches the referenced spec in the knowledge base.
2. All gates in §7 pass locally.
3. If any Rust IPC type changed: `gen:types` re-run and `bindings.ts` committed.
4. New error paths follow `09_ERROR_AND_LOGGING.md`; new UI strings are i18n keys.
5. Performance-sensitive changes (list, search, sync, embedding): run
   `cargo xtask bench --smoke` and check nothing regresses past its threshold;
   the nightly Tier-A run is the authoritative gate.
6. `CHANGELOG.md` updated; CI green.
7. For release-bound batches: `scripts/smoke_v04.sh` green and
   `scripts/release_check.sh <tag>` exit 0 before the tag is pushed.

## 9. Contributor License Agreement (CLA)

SeekerMail is dual-licensed: the code here is open source under AGPL-3.0, and a
separate commercial license is offered to organizations that cannot use the AGPL.
To be able to offer both, the project must hold a clear license to every
contribution. Therefore **all contributors must agree to the Contributor License
Agreement in [`CLA.md`](CLA.md) before a pull request can be merged.**

- You keep the copyright to your contribution; you grant vivbase a broad license
  to use and relicense it (including under the commercial license).
- Agreement is recorded automatically: the CLA Assistant bot comments on your
  first pull request and you accept by replying once. Returning contributors are
  remembered.
- This only affects outside contributors opening PRs; it changes nothing about
  how you run or self-host the AGPL code.

## 10. License compliance and building installers

Two gates keep the dependency tree clean - a single GPL/AGPL dependency would
force the whole app copyleft and break the open-core model:

```bash
node scripts/ci/check-licenses.mjs                    # npm license gate
cargo install cargo-deny --locked                     # one-time
cargo deny --manifest-path src-tauri/Cargo.toml check # Rust gate (desktop app)
cargo deny --manifest-path xtask/Cargo.toml     check # Rust gate (tooling)
```

Both also run in CI (`.github/workflows/license-check.yml`).

To build a release installer that bundles the third-party notices:

```bash
pnpm install --frozen-lockfile
cargo install cargo-about --locked --features cli   # one-time, for the notices
bash scripts/ci/generate-notice.sh         # writes THIRD-PARTY-NOTICES.md
pnpm tauri:build                           # DMG/app in src-tauri/target/release/bundle/
```
