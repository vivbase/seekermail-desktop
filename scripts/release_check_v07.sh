#!/usr/bin/env bash
# v0.7 RC pre-tag checklist (T107 §3g). The final gate a Release Engineer runs
# before the PM pushes `v0.7.0-rc`. Verifies the automated preconditions are all
# satisfied; exits non-zero if any fail. Does NOT push the tag (PM, post-merge).
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
ok()  { echo "  ✓ $1"; }
bad() { echo "  [FAIL] $1"; fail=$((fail+1)); }

echo "v0.7 RC pre-tag checklist"

# 1. AI safety gate green (T104). Generate a fresh report then gate it.
echo "▶ 1. AI safety gate (T104)"
if command -v cargo >/dev/null; then
  if cargo xtask safety-run --out /tmp/rc_safety_report.json >/tmp/rc_safety_run.log 2>&1 \
     && cargo xtask safety-gate --report /tmp/rc_safety_report.json >/tmp/rc_safety_gate.log 2>&1; then
    ok "safety-gate green (misfire < 5%, downgrade 10–30%)"
  else
    bad "safety-gate not green (see /tmp/rc_safety_gate.log)"
  fi
else
  bad "cargo not available — cannot run the safety gate"
fi

# 2. Compliance tests (T103).
echo "▶ 2. Compliance tests (T103)"
if command -v cargo >/dev/null; then
  cargo test --manifest-path src-tauri/Cargo.toml --test compliance >/tmp/rc_compliance.log 2>&1 \
    && ok "no-proxy egress + log-safety green" || bad "compliance failed (see /tmp/rc_compliance.log)"
else
  bad "cargo not available — cannot run compliance tests"
fi

# 3. Full RC smoke (automated portion; E2E confirmed separately).
echo "▶ 3. RC smoke (automated portion)"
bash scripts/smoke_v07_rc.sh >/tmp/rc_smoke.log 2>&1
# Exit 1 with only E2E pending is acceptable here (E2E is human-confirmed); a
# hard failure prints [FAIL] lines in the log.
if grep -q "\[FAIL\]" /tmp/rc_smoke.log; then
  bad "smoke_v07_rc.sh has hard failures (see /tmp/rc_smoke.log)"
else
  ok "smoke automated gates clean (confirm E2E with SMOKE_E2E=1 before tag)"
fi

# 4. Backend + xtask unit tests.
echo "▶ 4. Unit tests"
if command -v cargo >/dev/null; then
  cargo test --manifest-path src-tauri/Cargo.toml >/tmp/rc_cargo.log 2>&1 \
    && ok "cargo test (src-tauri)" || bad "cargo test failed (see /tmp/rc_cargo.log)"
  cargo test --manifest-path xtask/Cargo.toml >/tmp/rc_xtask.log 2>&1 \
    && ok "cargo test (xtask)" || bad "xtask tests failed (see /tmp/rc_xtask.log)"
fi

# 5. Working tree clean.
echo "▶ 5. Clean working tree"
if [ -z "$(git status --porcelain 2>/dev/null)" ]; then
  ok "git status clean"
else
  bad "uncommitted changes present — commit or stash before tagging"
fi

# 6. CHANGELOG has a non-empty 0.7.0-rc section.
echo "▶ 6. CHANGELOG"
if grep -qE '\[0\.7\.0-rc\]' CHANGELOG.md; then
  ok "CHANGELOG has a [0.7.0-rc] section"
else
  bad "CHANGELOG missing the [0.7.0-rc] section (PM finalizes at tag time)"
fi

# 7. Release evidence archived (human-produced; presence-checked).
echo "▶ 7. Release evidence (docs/releases/)"
for f in v0.7.0-rc_safety_report.json v0.7.0-rc_noproxy_check.png v0.7.0-rc_e5_blind_test.md v0.7.0-rc_e7_csv_sample.csv; do
  [ -f "docs/releases/$f" ] && ok "evidence: $f" || echo "  • PENDING evidence (release engineer): docs/releases/$f"
done

echo ""; echo "──────────────────────────────────────────────"
if [ "$fail" -gt 0 ]; then
  echo "✗ RC pre-tag checklist FAILED ($fail) — do not push v0.7.0-rc."
  exit 1
fi
echo "✓ RC automated preconditions met. Confirm E2E + operational metrics (WTP, full-auto opt-in) in RC notes, then PM tags v0.7.0-rc."
exit 0
