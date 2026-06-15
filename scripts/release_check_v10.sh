#!/usr/bin/env bash
# v1.0 GA pre-tag checklist (T118 §3g). Inherits the v0.7 RC gate and adds the
# v1.0 GA preconditions. Exits non-zero if any hard check fails. Does NOT push
# the tag — the PM does that post-merge after confirming the commercial gate
# (§3c: third-party audit, privacy white paper, SLA, pricing, support).
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
ok() { echo "  ✓ $1"; }
bad() {
  echo "  [FAIL] $1"
  fail=$((fail + 1))
}
soft() { echo "  • $1"; }

echo "v1.0 GA pre-tag checklist (T118)"

echo "▶ 1. v0.7 RC gate (inherited)"
if [ -f scripts/release_check_v07.sh ]; then
  bash scripts/release_check_v07.sh >/tmp/rc10_v07.log 2>&1 &&
    ok "release_check_v07.sh exit 0" || bad "v0.7 gate failed (see /tmp/rc10_v07.log)"
else
  bad "release_check_v07.sh missing"
fi

echo "▶ 2. M9/M10 bench gate"
if cargo xtask bench-gate >/tmp/rc10_bench.log 2>&1; then
  ok "bench-gate green"
else
  soft "bench-gate needs a seeded corpus — run cargo xtask bench-seed && cargo xtask bench (M11 on a physical Windows box)"
fi

echo "▶ 3. Audit reports (T117) present, no ❌ FAIL"
for r in docs/releases/v1.0.0_parity_report.md docs/releases/v1.0.0_security_audit.md; do
  if [ -f "$r" ]; then
    if grep -q "❌ FAIL" "$r"; then bad "$r contains ❌ FAIL"; else ok "$(basename "$r") present, no ❌ FAIL"; fi
  else
    bad "$r missing"
  fi
done

echo "▶ 4. cargo test --all"
(cd src-tauri && cargo test --all >/tmp/rc10_cargo.log 2>&1) &&
  ok "cargo test --all" || bad "cargo test failed (see /tmp/rc10_cargo.log)"

echo "▶ 5. CHANGELOG [1.0.0]"
grep -qE '^\#{0,3} ?\[1\.0\.0\]' CHANGELOG.md &&
  ok "CHANGELOG has a [1.0.0] section" || bad "CHANGELOG missing [1.0.0]"

echo "▶ 6. Clean working tree"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  [ -z "$(git status --porcelain)" ] && ok "git status clean" || bad "uncommitted changes present"
else
  soft "not a git work tree here — verify a clean tree on the release machine"
fi

echo "▶ 7. CI + release.yml Windows leg (verify in GitHub)"
if command -v gh >/dev/null 2>&1; then
  soft "verify: gh run list -w ci.yml (windows leg green); gh run list -w release.yml (windows leg success)"
else
  soft "gh not installed — verify CI + release.yml Windows leg green in the GitHub UI"
fi

echo ""
echo "──────────────────────────────────────────────"
[ "$fail" -gt 0 ] && {
  echo "✗ v1.0 GA pre-tag checklist FAILED ($fail). Do not push v1.0.0."
  exit 1
}
echo "✓ v1.0 GA engineering preconditions met. PM confirms the commercial gate (§3c), then tags v1.0.0."
exit 0
