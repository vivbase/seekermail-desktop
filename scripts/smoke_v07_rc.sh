#!/usr/bin/env bash
# v0.7 RC comprehensive smoke gate (T107 §3e). Closes out the v0.5–v0.7 AI batch:
# E3 · E4 · E5 · E7 · F4 · F5, with the T104 AI-safety gate run inline and the
# T103 compliance gate inherited. Automated gates run unattended; app/live RC
# cases confirm interactively under SMOKE_E2E=1. Failure exits non-zero — the PM
# pushes v0.7.0-rc after merge only on a green gate.
#
#   bash scripts/smoke_v07_rc.sh
#   SMOKE_E2E=1 bash scripts/smoke_v07_rc.sh
#   SMOKE_SKIP_RUST=1 bash …            # frontend + (no cargo) — RC needs rust
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

echo "SeekerMail v0.7 RC smoke gate (E3 · E4 · E5 · E7 · F4 · F5)"
command -v node >/dev/null || { echo "missing node"; exit 2; }
command -v pnpm >/dev/null || { echo "missing pnpm"; exit 2; }

# ── 1. Frontend gates ─────────────────────────────────────────────────────────
step "1. Frontend gates (check + tests)"
pnpm check >/tmp/smoke07_check.log 2>&1 && ok "pnpm check" || bad "pnpm check (see /tmp/smoke07_check.log)"
pnpm test  >/tmp/smoke07_vitest.log 2>&1 && ok "pnpm test" || bad "pnpm test (see /tmp/smoke07_vitest.log)"

# ── 2. Rust gates + compliance ────────────────────────────────────────────────
if [ "${SMOKE_SKIP_RUST:-0}" = "1" ]; then
  echo ""; echo "▶ 2. Rust gates — SKIPPED (SMOKE_SKIP_RUST=1; RC must run them)"
else
  command -v cargo >/dev/null || { echo "missing cargo"; exit 2; }
  step "2. Rust gates (fmt + test + compliance)"
  ( cd src-tauri && cargo fmt --all --check >/tmp/smoke07_fmt.log 2>&1 ) && ok "cargo fmt --check" || bad "cargo fmt"
  ( cd src-tauri && cargo test >/tmp/smoke07_cargo.log 2>&1 ) && ok "cargo test" || bad "cargo test (see /tmp/smoke07_cargo.log)"
  ( cargo test --manifest-path src-tauri/Cargo.toml --test compliance >/tmp/smoke07_compliance.log 2>&1 ) \
    && ok "compliance (no-proxy + log-safety, T103)" || bad "compliance (see /tmp/smoke07_compliance.log)"

  # ── 3. AI safety harness (T104) — seed → run → gate, inline ──────────────────
  step "3. AI safety harness (T104): misfire < 5%, sensitive-downgrade 10–30%"
  cargo test --manifest-path xtask/Cargo.toml safety >/tmp/smoke07_xtask.log 2>&1 \
    && ok "xtask safety unit tests" || bad "xtask safety tests (see /tmp/smoke07_xtask.log)"
  cargo xtask safety-seed >/tmp/smoke07_seed.log 2>&1 && ok "safety-seed" || bad "safety-seed"
  cargo xtask safety-run --out /tmp/smoke07_safety_report.json >/tmp/smoke07_run.log 2>&1 \
    && ok "safety-run produced a report" || bad "safety-run (see /tmp/smoke07_run.log)"
  cargo xtask safety-gate --report /tmp/smoke07_safety_report.json >/tmp/smoke07_gate.log 2>&1 \
    && ok "safety-gate GREEN" || bad "safety-gate RED (see /tmp/smoke07_gate.log)"
fi

# ── 4. v0.7 feature presence spot-checks (E3/E4/E5/E7/F4/F5) ──────────────────
step "4. v0.7 feature spot checks"
[ -f src-tauri/src/ai/pipeline/e3_pipeline.rs ] && ok "E3: full-auto pipeline present" || bad "E3 missing"
[ -f src-tauri/src/ai/pipeline/e4_classifier.rs ] && ok "E4: sensitive pre-scan present" || bad "E4 missing"
[ -d src-tauri/src/ai/style ] && ok "E5: style learning present" || bad "E5 missing"
[ -d src-tauri/src/ai/audit ] && ok "E7: audit log present" || bad "E7 missing"
grep -q 'get_provider_matrix' src-tauri/src/lib.rs && ok "F4: provider matrix command present" || bad "F4 missing"
[ -f src-tauri/src/ai/fallback.rs ] && ok "F5: offline fallback present" || bad "F5 missing"

# ── 5. RC end-to-end (app required; mock provider/SMTP) ───────────────────────
step "5. RC end-to-end (app required; SEEKERMAIL_AI_MOCK=1, short undo window)"
confirm "Full-Auto account → 3 ordinary mails → pass self-check → send_queued (undo window) → sent_auto"
confirm "2 sensitive mails (amount) → demoted to drafts → Pending draft cards visible"
confirm "E7 auto-reply log → 3 sent_auto rows → filter by account + time → export CSV non-empty"
confirm "F4 matrix → two accounts on different providers → inference routes to the correct provider"
confirm "F5 offline → inject AI_PROVIDER_UNREACHABLE → pipeline downgrades to draft → no mail lost"
confirm "Undo window → start send_queued → undo → mail becomes a Pending draft (undo_canceled)"
confirm "Trust downgrade → inject 3 mis-send reports in 7 days → account auto-demotes E3→E2, logged to E7"

echo ""; echo "──────────────────────────────────────────────"
echo "v0.7 RC smoke gate: $pass passed, $fail failed, $manual_pending E2E pending"
[ "$fail" -gt 0 ] && { echo "✗ FAILURES — do not push v0.7.0-rc."; exit 1; }
[ "$manual_pending" -gt 0 ] && { echo "• Automated gates green; re-run with SMOKE_E2E=1 before tagging."; exit 1; }
echo "✓ v0.7 RC gate GREEN — run scripts/release_check_v07.sh."
exit 0
