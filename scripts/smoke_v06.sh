#!/usr/bin/env bash
# v0.6 Beta end-to-end smoke gate (T106 §3d). Covers D2 · E2 · E6 · I3 · I4 and
# the mis-send protection drill (E2 + E4 interception, 100%). Automated gates run
# unattended; app/live cases confirm interactively under SMOKE_E2E=1. Failure
# exits non-zero — the PM pushes v0.6.0-beta after merge only on a green gate.
#
#   bash scripts/smoke_v06.sh
#   SMOKE_E2E=1 bash scripts/smoke_v06.sh
#   SMOKE_SKIP_RUST=1 bash …
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
    echo "  • PENDING (needs SMOKE_E2E=1 + app): $1"; manual_pending=$((manual_pending+1))
  fi
}

echo "SeekerMail v0.6 beta smoke gate (D2 · E2 · E6 · I3 · I4)"
command -v node >/dev/null || { echo "missing node"; exit 2; }
command -v pnpm >/dev/null || { echo "missing pnpm"; exit 2; }

# ── 1. Frontend gates ─────────────────────────────────────────────────────────
step "1. Frontend gates (check + tests)"
pnpm check >/tmp/smoke06_check.log 2>&1 && ok "pnpm check" || bad "pnpm check (see /tmp/smoke06_check.log)"
pnpm test  >/tmp/smoke06_vitest.log 2>&1 && ok "pnpm test" || bad "pnpm test (see /tmp/smoke06_vitest.log)"

# ── 2. Rust gates incl. compliance + E2 filter accuracy ──────────────────────
if [ "${SMOKE_SKIP_RUST:-0}" = "1" ]; then
  echo ""; echo "▶ 2. Rust gates — SKIPPED (SMOKE_SKIP_RUST=1)"
else
  command -v cargo >/dev/null || { echo "missing cargo"; exit 2; }
  step "2. Rust gates (fmt + test + compliance)"
  ( cd src-tauri && cargo fmt --all --check >/tmp/smoke06_fmt.log 2>&1 ) && ok "cargo fmt --check" || bad "cargo fmt"
  ( cd src-tauri && cargo test >/tmp/smoke06_cargo.log 2>&1 ) && ok "cargo test" || bad "cargo test (see /tmp/smoke06_cargo.log)"
  ( cargo test --manifest-path src-tauri/Cargo.toml --test compliance >/tmp/smoke06_compliance.log 2>&1 ) \
    && ok "compliance (T103, inherited)" || bad "compliance (see /tmp/smoke06_compliance.log)"
fi

# ── 3. v0.6 feature presence spot-checks (I3/I4 + E2/E6/D2) ───────────────────
step "3. v0.6 feature spot checks"
[ -f src-tauri/migrations/009_mail_processing_status.sql ] && ok "I3: ai_processing_status migration (009)" || bad "I3 migration missing"
[ -f src-tauri/src/ai/pipeline/i3_stage.rs ] && ok "I3: proactive query stage present" || bad "i3_stage missing"
[ -f src-tauri/src/ai/qa_card.rs ] && ok "I4: QA card schema present" || bad "qa_card missing"
grep -q 'answer_query' src-tauri/src/lib.rs && ok "I3: answer/skip commands registered" || bad "answer_query missing"
[ -f src/components/pending/DecisionCard.tsx ] && ok "I4: Pending DecisionCard present" || bad "DecisionCard missing"
[ -f src/components/pending/DraftCard.tsx ] && ok "E6: Pending DraftCard present" || bad "DraftCard missing"
grep -q 'analyze_sales_context' src-tauri/src/lib.rs && ok "D2: sales analysis command present" || bad "D2 missing"

# ── 4. Mis-send protection drill (T106 §3b — E2 + E4, 100% interception) ──────
step "4. Mis-send protection drill (E2 + E4)"
confirm "Inject 3 mis-trigger mails (amount / PDF attachment / important contact) under a Semi-Auto account → all 3 land as DRAFTS, none auto-sent (100%)"
echo "  • Archive the drill screenshot to docs/releases/v0.6.0-beta_mis_send_drill.png"

# ── 5. E2E (app + mock provider; live-confirmed) ─────────────────────────────
step "5. End-to-end cases (app required; SEEKERMAIL_AI_MOCK=1)"
confirm "Semi-Auto account → import 5 fixtures (2 reply-needed + 2 marketing + 1 CC) → 2 drafts, 3 skipped"
confirm "Pending → draft card count = 2 → full-keyboard 'send' the first (Tab/Enter, no mouse)"
confirm "Trigger an I3 T1 query → Pending decision card appears → pick an option → card resolves"
confirm "Trigger an I3 T4 query → red non-dismissable banner appears; only Resolve clears it"
confirm "Settings → Agents → D2 analyse a negotiation email (mock) → strategy card returned"

echo ""; echo "──────────────────────────────────────────────"
echo "v0.6 smoke gate: $pass passed, $fail failed, $manual_pending E2E pending"
[ "$fail" -gt 0 ] && { echo "✗ FAILURES — do not push v0.6.0-beta."; exit 1; }
[ "$manual_pending" -gt 0 ] && { echo "• Automated gates green; re-run with SMOKE_E2E=1 before tagging."; exit 1; }
echo "✓ v0.6 gate GREEN."
exit 0
