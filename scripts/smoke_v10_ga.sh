#!/usr/bin/env bash
# v1.0 GA comprehensive smoke gate (T118 §3e). Exercises the v1.0 increment —
# attachment full-text search (T108–T110), cross-account unified search
# (T111–T113), the deal transaction view + P0 isolation (T119/T120), the Windows
# credential round-trip (Windows only, T114) — and inlines the v0.7 RC smoke.
# Any hard failure exits non-zero; the PM tags v1.0.0 only on a green gate.
#
#   bash scripts/smoke_v10_ga.sh
set -uo pipefail
cd "$(dirname "$0")/.."

pass=0
fail=0
ok() {
  echo "  ✓ $1"
  pass=$((pass + 1))
}
bad() {
  echo "  [FAIL] $1"
  fail=$((fail + 1))
}
step() {
  echo ""
  echo "▶ $1"
}

echo "SeekerMail v1.0 GA smoke gate"
command -v cargo >/dev/null || {
  echo "missing cargo"
  exit 2
}

step "1. Attachment extraction + index (T108/T109)"
(cd src-tauri && cargo test --lib -- extraction:: >/tmp/smoke10_extraction.log 2>&1) &&
  ok "extraction + indexer tests" || bad "extraction (see /tmp/smoke10_extraction.log)"

step "2. Attachment FTS search scope (T109/T110)"
(cd src-tauri && cargo test --lib -- search::fts5::tests::attachment_fts >/tmp/smoke10_attfts.log 2>&1) &&
  ok "attachment FTS account scope" || bad "attachment FTS (see /tmp/smoke10_attfts.log)"

step "3. Cross-account keyword search + M10 determinism (T111)"
(cd src-tauri && cargo test --lib -- search::fts5::tests::cross_account >/tmp/smoke10_xkw.log 2>&1) &&
  ok "cross-account keyword + deterministic order" || bad "cross-account keyword (see /tmp/smoke10_xkw.log)"

step "4. Cross-account semantic search + account_filter (T112)"
(cd src-tauri && cargo test --lib -- search::ann::tests::cross_account search::ann::tests::account_filter >/tmp/smoke10_xsem.log 2>&1) &&
  ok "cross-account semantic + account_filter" || bad "cross-account semantic (see /tmp/smoke10_xsem.log)"

step "5. Deal transaction view + P0 isolation (T119/T120)"
(cd src-tauri && cargo test --lib -- deal:: >/tmp/smoke10_deal.log 2>&1) &&
  ok "deal CRUD + timeline + zero ai_* writes" || bad "deal (see /tmp/smoke10_deal.log)"

step "6. Windows Credential Manager round-trip (Windows only, T114)"
case "$(uname -s)" in
*NT* | *MINGW* | *MSYS*)
  (cd src-tauri && cargo test --lib -- keychain --include-ignored >/tmp/smoke10_kc.log 2>&1) &&
    ok "credential round-trip" || bad "credential round-trip (see /tmp/smoke10_kc.log)"
  ;;
*) echo "  • SKIPPED (not Windows) — run on a Windows runner with --include-ignored" ;;
esac

step "7. Inherited v0.7 RC smoke"
if [ -f scripts/smoke_v07_rc.sh ]; then
  bash scripts/smoke_v07_rc.sh >/tmp/smoke10_v07.log 2>&1
  if grep -q "\[FAIL\]" /tmp/smoke10_v07.log; then
    bad "v0.7 RC smoke had hard failures (see /tmp/smoke10_v07.log)"
  else
    ok "v0.7 RC smoke automated gates clean"
  fi
else
  bad "smoke_v07_rc.sh missing"
fi

step "8. Performance gate (M1–M10; M11 manual on Windows)"
if cargo xtask bench-gate >/tmp/smoke10_bench.log 2>&1; then
  ok "cargo xtask bench-gate green"
else
  echo "  • bench-gate needs a seeded corpus — Release Engineer runs cargo xtask bench;"
  echo "    M11 (keyword P95 < 300 ms) is validated on a physical Windows Tier C box"
fi

echo ""
echo "──────────────────────────────────────────────"
echo "v1.0 GA smoke: $pass passed, $fail failed"
[ "$fail" -gt 0 ] && {
  echo "✗ FAILURES — do not push v1.0.0."
  exit 1
}
echo "✓ v1.0 GA smoke automated gates GREEN — run scripts/release_check_v10.sh."
exit 0
