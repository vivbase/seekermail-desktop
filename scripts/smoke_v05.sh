#!/usr/bin/env bash
# v0.5 Beta end-to-end smoke gate (T105 §3d). Mirrors scripts/smoke_v04.sh:
# automated gates run unattended; live-account / app-run E2E cases are confirmed
# interactively under SMOKE_E2E=1. Any failure exits non-zero — DO NOT push the
# tag on failure (the PM pushes v0.5.0-beta after merge; this script gates it).
#
#   bash scripts/smoke_v05.sh             # automated gates (+ E2E listed pending)
#   SMOKE_E2E=1 bash scripts/smoke_v05.sh # + interactive E2E confirmation
#   SMOKE_SKIP_RUST=1 bash …              # frontend-only subset (no cargo)
set -uo pipefail
cd "$(dirname "$0")/.."

pass=0; fail=0; manual_pending=0
ok()   { echo "  ✓ $1"; pass=$((pass+1)); }
bad()  { echo "  [FAIL] $1"; fail=$((fail+1)); }
step() { echo ""; echo "▶ $1"; }
confirm() {
  if [ "${SMOKE_E2E:-0}" = "1" ]; then
    read -r -p "  E2E · $1 — passed? [y/N] " reply
    case "$reply" in [yY]*) ok "E2E: $1" ;; *) bad "E2E: $1" ;; esac
  else
    echo "  • PENDING (needs SMOKE_E2E=1 + app/live account): $1"; manual_pending=$((manual_pending+1))
  fi
}

echo "SeekerMail v0.5 beta smoke gate (F1–F3 · D1 · E1 · I1 · I2)"
command -v node >/dev/null || { echo "missing node"; exit 2; }
command -v pnpm >/dev/null || { echo "missing pnpm"; exit 2; }

# ── 1. Frontend gates ─────────────────────────────────────────────────────────
step "1. Frontend gates (check + tests)"
pnpm check  >/tmp/smoke05_check.log  2>&1 && ok "pnpm check"  || bad "pnpm check (see /tmp/smoke05_check.log)"
pnpm test   >/tmp/smoke05_vitest.log 2>&1 && ok "pnpm test"   || bad "pnpm test (see /tmp/smoke05_vitest.log)"

# ── 2. Rust gates incl. T103 compliance ───────────────────────────────────────
if [ "${SMOKE_SKIP_RUST:-0}" = "1" ]; then
  echo ""; echo "▶ 2. Rust gates — SKIPPED (SMOKE_SKIP_RUST=1)"
else
  command -v cargo >/dev/null || { echo "missing cargo"; exit 2; }
  step "2. Rust gates (fmt + test + T103 compliance)"
  ( cd src-tauri && cargo fmt --all --check >/tmp/smoke05_fmt.log 2>&1 ) && ok "cargo fmt --check" || bad "cargo fmt"
  ( cd src-tauri && cargo test >/tmp/smoke05_cargo.log 2>&1 ) && ok "cargo test" || bad "cargo test (see /tmp/smoke05_cargo.log)"
  ( cargo test --manifest-path src-tauri/Cargo.toml --test compliance >/tmp/smoke05_compliance.log 2>&1 ) \
    && ok "compliance (no-proxy egress + log-safety, T103)" || bad "compliance (see /tmp/smoke05_compliance.log)"
fi

# ── 3. v0.5 feature presence spot-checks (I1/I2 + F/D/E surfaces) ─────────────
step "3. v0.5 feature spot checks"
[ -f src-tauri/migrations/008_im_messages.sql ] && ok "I2: im_messages migration (008)" || bad "I2 migration missing"
grep -q 'set_primary_account' src-tauri/src/lib.rs && ok "I1: set_primary_account registered" || bad "I1 command missing"
grep -q 'get_agent_statuses' src-tauri/src/lib.rs && ok "I2: agent presence command registered" || bad "agent presence missing"
[ -f src/components/agent/TeamChannel.tsx ] && ok "I2: TEAM channel UI present" || bad "TeamChannel missing"
[ -f src/components/agent/AgentAvatar.tsx ] && ok "I2: deterministic agent avatar present" || bad "AgentAvatar missing"
grep -q '/settings/ai' src/App.tsx && ok "F1–F3: AI provider settings routed" || bad "AI settings route missing"
[ -f docs/compliance/noproxy_check_sop.md ] && ok "compliance: no-proxy SOP present" || bad "no-proxy SOP missing"

# ── 4. E2E (app + mock provider; live-confirmed) ─────────────────────────────
step "4. End-to-end cases (app required; SEEKERMAIL_AI_MOCK=1)"
confirm "Add an OpenAI provider (mock key) → Test Connection passes"
confirm "L2 reading view → AI Reply → pick Friendly style → draft generated → compose prefilled"
confirm "Team channel → send '@<agent>: what is the renewal clause?' → agent reply card appears"
confirm "Settings → Agents → switch an account to Manual and confirm"
confirm "Settings → Data Flow → page reachable, shows the current provider"

echo ""; echo "──────────────────────────────────────────────"
echo "v0.5 smoke gate: $pass passed, $fail failed, $manual_pending E2E pending"
[ "$fail" -gt 0 ] && { echo "✗ FAILURES — do not push v0.5.0-beta."; exit 1; }
[ "$manual_pending" -gt 0 ] && { echo "• Automated gates green; re-run with SMOKE_E2E=1 before tagging."; exit 1; }
echo "✓ v0.5 gate GREEN."
exit 0
