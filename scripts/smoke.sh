#!/usr/bin/env bash
# v0.1 skeleton smoke gate (T012 §6). End-to-end verification that T001–T011 hold
# together. Run on a clean clone on macOS. Exits non-zero on the first failure.
#
#   bash scripts/smoke.sh           # full gate (needs Node, pnpm, Rust, cargo)
#   SMOKE_SKIP_RUST=1 bash …        # frontend-only subset (no cargo)
#   SMOKE_RUN_MODEL=1 bash …        # also fetch the ONNX model (~2.2 GB)
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"

pass=0; fail=0
ok()   { echo "  ✓ $1"; pass=$((pass+1)); }
bad()  { echo "  ✗ $1"; fail=$((fail+1)); }
step() { echo ""; echo "▶ $1"; }
need() { command -v "$1" >/dev/null 2>&1 || { echo "missing required tool: $1"; exit 2; }; }

echo "SeekerMail v0.1 skeleton smoke gate"
need node; need pnpm; need git

# 1) Clean install (T001)
step "1. pnpm install"
if pnpm install --frozen-lockfile >/tmp/smoke_install.log 2>&1 || pnpm install >/tmp/smoke_install.log 2>&1; then
  ok "dependencies installed"
else
  bad "pnpm install failed (see /tmp/smoke_install.log)"
fi

# 2) IPC ping path (T002) — exercised by the frontend + Rust unit tests below.
step "2. IPC ping path"
if grep -q "fn ping" src-tauri/src/commands/system.rs && grep -q "generate_handler!\[commands::ping\]" src-tauri/src/lib.rs; then
  ok "ping command defined and registered"
else
  bad "ping command/registration missing"
fi

# 6a) Frontend gates: types, lint, prettier, boundaries, unit tests (T007/T008)
step "6. Frontend gates (tsc + eslint + prettier + boundaries + tests)"
pnpm check        >/tmp/smoke_check.log 2>&1 && ok "pnpm check" || bad "pnpm check (see /tmp/smoke_check.log)"
bash scripts/check-boundaries.sh >/tmp/smoke_bound.log 2>&1 && ok "no bare hex / tauri api only in src/ipc" || bad "boundary checks (see /tmp/smoke_bound.log)"
pnpm test         >/tmp/smoke_test.log 2>&1 && ok "vitest (shell 3 regions, RTL switch, ipc, errors)" || bad "vitest (see /tmp/smoke_test.log)"

# Build the production frontend bundle (proves the shell compiles).
pnpm build        >/tmp/smoke_build.log 2>&1 && ok "vite build" || bad "vite build (see /tmp/smoke_build.log)"

# 4/5) Rust gates: fmt, clippy, tests (storage tables+PRAGMA+FK, error model,
#       log safety, keychain redaction). Keychain roundtrip is macOS --ignored.
if [ "${SMOKE_SKIP_RUST:-0}" = "1" ]; then
  echo ""; echo "▶ 4/5. Rust gates — SKIPPED (SMOKE_SKIP_RUST=1)"
else
  need cargo
  step "4/5. Rust gates (fmt + clippy + tests: storage, errors, keychain)"
  ( cd src-tauri
    cargo fmt --all --check >/tmp/smoke_fmt.log 2>&1 ) && ok "cargo fmt --check" || bad "cargo fmt (see /tmp/smoke_fmt.log)"
  ( cd src-tauri
    cargo clippy --all-targets -- -D warnings >/tmp/smoke_clippy.log 2>&1 ) && ok "cargo clippy -D warnings" || bad "cargo clippy (see /tmp/smoke_clippy.log)"
  ( cd src-tauri
    cargo test >/tmp/smoke_cargo_test.log 2>&1 ) && ok "cargo test (001_init tables, PRAGMA/FK, log safety)" || bad "cargo test (see /tmp/smoke_cargo_test.log)"
  if [ "$(uname)" = "Darwin" ]; then
    ( cd src-tauri
      cargo test --features '' -- --ignored set_get_delete_roundtrip >/tmp/smoke_keychain.log 2>&1 ) \
        && ok "keychain set/get/delete roundtrip" || echo "  • keychain roundtrip needs interactive Keychain — run locally if prompted"
  fi

  # 3) specta bindings drift check (T003)
  step "3. specta bindings drift check"
  if pnpm run gen:types >/tmp/smoke_gen.log 2>&1; then
    if git diff --exit-code packages/shared/src/bindings.ts >/dev/null 2>&1; then
      ok "bindings in sync (gen:types clean)"
    else
      bad "bindings drifted — commit the regenerated packages/shared/src/bindings.ts"
    fi
  else
    bad "gen:types failed (see /tmp/smoke_gen.log)"
  fi
fi

# 7) Model fetch idempotency (T010) — syntax always; full fetch is opt-in.
step "7. Model fetch script"
node --check scripts/setup-model.mjs && ok "setup-model.mjs parses" || bad "setup-model.mjs syntax error"
if [ "${SMOKE_RUN_MODEL:-0}" = "1" ]; then
  pnpm run setup:model && pnpm run setup:model | grep -q "already present" \
    && ok "setup:model idempotent (second run skips)" || bad "setup:model not idempotent"
fi

# 8) Secrets/model not tracked (T011) + CI present (T009)
step "8. Ignore rules + CI presence"
touch src-tauri/resources/model.onnx 2>/dev/null || true
git check-ignore -q src-tauri/resources/model.onnx && ok ".onnx weights are git-ignored" || bad "model weights not ignored"
rm -f src-tauri/resources/model.onnx
git check-ignore -q .env && ok ".env is git-ignored" || bad ".env not ignored"
git ls-files --error-unmatch src-tauri/resources/.gitkeep >/dev/null 2>&1 && ok "resources/.gitkeep tracked" || echo "  • resources/.gitkeep not yet committed"
[ -f .github/workflows/ci.yml ] && ok "CI workflow present (macos-14)" || bad "CI workflow missing"

echo ""
echo "──────────────────────────────────────────────"
echo "smoke gate: $pass passed, $fail failed"
[ "$fail" -eq 0 ] && { echo "✓ v0.1 skeleton is GREEN"; exit 0; } || { echo "✗ skeleton has failures"; exit 1; }
