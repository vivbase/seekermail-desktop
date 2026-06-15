#!/usr/bin/env bash
# v0.4 Beta end-to-end smoke gate (T057 §3b). The v0.4 counterpart of
# scripts/smoke.sh (T012): automated gates run unattended; the five E2E cases
# from planning/05 §4.2 that need a live mail account run interactively under
# SMOKE_E2E=1 with the Release Engineer confirming each step. Any failure exits
# non-zero and the tag MUST NOT be pushed (T057 §6).
#
#   bash scripts/smoke_v04.sh             # automated gates only (E2E listed as pending)
#   SMOKE_E2E=1 bash scripts/smoke_v04.sh # + interactive E2E confirmation
#   SMOKE_SKIP_RUST=1 bash …              # frontend-only subset (no cargo)
set -uo pipefail
cd "$(dirname "$0")/.."

pass=0; fail=0; manual_pending=0
ok()   { echo "  ✓ $1"; pass=$((pass+1)); }
bad()  { echo "  [FAIL] $1"; fail=$((fail+1)); }
step() { echo ""; echo "▶ $1"; }
need() { command -v "$1" >/dev/null 2>&1 || { echo "missing required tool: $1"; exit 2; }; }

# Interactive yes/no for E2E cases (SMOKE_E2E=1); otherwise listed as pending.
confirm() {
  local prompt="$1"
  if [ "${SMOKE_E2E:-0}" = "1" ]; then
    read -r -p "  E2E · ${prompt} — passed? [y/N] " reply
    case "$reply" in
      [yY]*) ok "E2E: $prompt" ;;
      *) bad "E2E: $prompt" ;;
    esac
  else
    echo "  • PENDING (needs SMOKE_E2E=1 + live account): $prompt"
    manual_pending=$((manual_pending+1))
  fi
}

echo "SeekerMail v0.4 beta smoke gate"
need node; need pnpm; need git

# ── 1. Automated build & test gates ───────────────────────────────────────────
step "1. Frontend gates (install + check + boundaries + tests + build)"
pnpm install --frozen-lockfile >/tmp/smoke04_install.log 2>&1 \
  || pnpm install >/tmp/smoke04_install.log 2>&1 \
  && ok "pnpm install" || bad "pnpm install (see /tmp/smoke04_install.log)"
pnpm check >/tmp/smoke04_check.log 2>&1 && ok "pnpm check" || bad "pnpm check (see /tmp/smoke04_check.log)"
bash scripts/check-boundaries.sh >/tmp/smoke04_bound.log 2>&1 \
  && ok "boundaries (no bare hex; tauri api only in src/ipc)" || bad "boundaries (see /tmp/smoke04_bound.log)"
pnpm test >/tmp/smoke04_vitest.log 2>&1 && ok "vitest" || bad "vitest (see /tmp/smoke04_vitest.log)"
pnpm build >/tmp/smoke04_build.log 2>&1 && ok "vite build" || bad "vite build (see /tmp/smoke04_build.log)"

if [ "${SMOKE_SKIP_RUST:-0}" = "1" ]; then
  echo ""; echo "▶ 2. Rust gates — SKIPPED (SMOKE_SKIP_RUST=1)"
else
  need cargo
  step "2. Rust gates (fmt + clippy + tests incl. export/wipe/reindex/settings)"
  ( cd src-tauri && cargo fmt --all --check >/tmp/smoke04_fmt.log 2>&1 ) \
    && ok "cargo fmt --check" || bad "cargo fmt (see /tmp/smoke04_fmt.log)"
  ( cd src-tauri && cargo clippy --all-targets -- -D warnings >/tmp/smoke04_clippy.log 2>&1 ) \
    && ok "cargo clippy -D warnings" || bad "cargo clippy (see /tmp/smoke04_clippy.log)"
  ( cd src-tauri && cargo test >/tmp/smoke04_cargo.log 2>&1 ) \
    && ok "cargo test (incl. T050–T053 suites)" || bad "cargo test (see /tmp/smoke04_cargo.log)"

  step "3. specta bindings drift check"
  if pnpm run gen:types >/tmp/smoke04_gen.log 2>&1; then
    git diff --exit-code packages/shared/src/bindings.ts >/dev/null 2>&1 \
      && ok "bindings in sync" || bad "bindings drifted — commit regenerated bindings.ts"
  else
    bad "gen:types failed (see /tmp/smoke04_gen.log)"
  fi

  step "4. Perf tooling (T055) — harness self-test + smoke run"
  ( cargo test --manifest-path xtask/Cargo.toml >/tmp/smoke04_xtask.log 2>&1 ) \
    && ok "xtask unit tests (seed determinism, gate exit codes)" || bad "xtask tests (see /tmp/smoke04_xtask.log)"
  ( cargo xtask bench-seed --count 1000 >/tmp/smoke04_seed.log 2>&1 ) \
    && ok "bench-seed 1k corpus" || bad "bench-seed (see /tmp/smoke04_seed.log)"
  ( cargo xtask bench --smoke --out /tmp/smoke04-bench.json >/tmp/smoke04_bench.log 2>&1 ) \
    && ok "bench --smoke produces a report" || bad "bench --smoke (see /tmp/smoke04_bench.log)"
fi

# ── 5. Release plumbing presence (T056) ──────────────────────────────────────
step "5. Release plumbing (T056)"
[ -f src-tauri/entitlements.plist ] && ok "entitlements.plist present" || bad "entitlements.plist missing"
[ -f src-tauri/deny.toml ] && ok "deny.toml present" || bad "deny.toml missing"
[ -f .github/workflows/release.yml ] && ok "release.yml present" || bad "release.yml missing"
[ -f .github/workflows/bench.yml ] && ok "bench.yml present" || bad "bench.yml missing"
grep -RInE 'BEGIN (RSA|EC|PGP|OPENSSH) PRIVATE KEY' --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=target . >/dev/null 2>&1 \
  && bad "a private key is committed somewhere — remove it" || ok "no private keys committed"

# ── 6. v0.4 new-feature spot checks (static, from T050–T054) ─────────────────
step "6. v0.4 feature spot checks"
SETTINGS_ROUTES=0
for r in accounts appearance privacy data about; do
  grep -q "\"$r\"" src/App.tsx && SETTINGS_ROUTES=$((SETTINGS_ROUTES+1))
done
[ "$SETTINGS_ROUTES" -eq 5 ] && ok "settings five categories routed" || bad "settings categories incomplete ($SETTINGS_ROUTES/5)"
grep -q 'block_known' src-tauri/src/commands/settings.rs \
  && ok "tracking protection defaults ON (block_known seed)" || bad "privacy default seed missing"
grep -q 'html.dark' src/styles/tokens.css \
  && ok "dark theme token overrides present" || bad "html.dark overrides missing"
# v0.4 ships the static no-AI placeholder; T069 (v0.5) supersedes it with the
# real AI routing section — accept either so the gate stays valid on both tags.
{ grep -q 'data_flow_no_ai_v04' src/routes/settings/data/data_flow/index.tsx \
  || grep -q 'AiRoutingSection' src/routes/settings/data/data_flow/index.tsx; } \
  && ok "data-flow panel AI section present (v0.4 placeholder or T069 routing)" || bad "data-flow AI section missing"
grep -q '"DELETE"' src/routes/settings/data/wipe/index.tsx \
  && ok "wipe wizard requires the typed DELETE guard" || bad "DELETE guard missing"

# ── 7. Five E2E cases (planning/05 §4.2) — live-account, human-confirmed ─────
step "7. End-to-end cases (live account required)"
confirm "Add a Gmail OAuth account → fetch 50 mails → L0 list renders → open the first mail in L2"
confirm "Keyword search 'invoice' returns results → clicking the first opens it"
confirm "Semantic search 'last email about contract' returns results → clicking opens it"
confirm "Reply to a mail → edit → send → message appears in Sent"
confirm "Settings → Data → Export mbox → file size > 0 and 'grep -c \"^From \"' matches the mail count"
confirm "Theme toggle applies instantly and survives an app restart"
confirm "Settings → Data → Data Flow shows the AI section (v0.4: 'No AI requests' notice; v0.5+: per-account AI routing, T069)"

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "──────────────────────────────────────────────"
echo "v0.4 smoke gate: $pass passed, $fail failed, $manual_pending E2E pending"
if [ "$fail" -gt 0 ]; then
  echo "✗ v0.4 gate has FAILURES — do not push the tag."
  exit 1
fi
if [ "$manual_pending" -gt 0 ]; then
  echo "• Automated gates green, but $manual_pending E2E case(s) unconfirmed."
  echo "  Re-run with SMOKE_E2E=1 against a live account before tagging."
  exit 1
fi
echo "✓ v0.4 gate is GREEN — proceed to scripts/release_check.sh."
exit 0
