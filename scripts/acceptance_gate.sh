#!/usr/bin/env bash
# Release Go/No-Go acceptance gate runner (TAS pillar S4).
#
# Runs the layered acceptance checks mapped to a given version's release gate
# (TEST_PLAN.md §5) and writes a one-page acceptance report with a sign-off box:
#   acceptance-report-<version>-<UTC-date>.md
#
# This is the SKELETON delivered in the P0/P1 phase: checks that can run locally
# run for real; checks that need real mailboxes, funded AI keys, or perf hardware
# are recorded as MANUAL so a human signs them off. As later phases land, flip a
# MANUAL row into a real command — the report shape stays the same.
#
# Usage:
#   scripts/acceptance_gate.sh <version>
#   <version> in: v0.4 | v0.5 | v0.6 | v0.7-rc | v1.0
# Example:
#   scripts/acceptance_gate.sh v0.6
#
# Exit code: 0 if no FAIL rows (MANUAL rows still require human sign-off in the
# report); non-zero if any FAIL.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="${1:-}"
case "$VERSION" in
  v0.4|v0.5|v0.6|v0.7-rc|v1.0) ;;
  *)
    echo "Usage: scripts/acceptance_gate.sh <v0.4|v0.5|v0.6|v0.7-rc|v1.0>" >&2
    exit 2
    ;;
esac

DATE_UTC="$(date -u '+%Y-%m-%d %H:%M UTC')"
REPORT="acceptance-report-${VERSION}-$(date -u '+%Y%m%d').md"

# Each row: "STATUS|LAYER|NAME|DETAIL"  (STATUS in PASS/FAIL/SKIP/MANUAL)
ROWS=()
FAILED=0

record() { ROWS+=("$1|$2|$3|$4"); [[ "$1" == "FAIL" ]] && FAILED=1; return 0; }

# run_check <layer> <name> <command...> : PASS if command exits 0, else FAIL.
run_check() {
  local layer="$1" name="$2"; shift 2
  echo "→ [$layer] $name"
  if "$@" >/tmp/ag_out 2>&1; then
    record PASS "$layer" "$name" "ok"
  else
    record FAIL "$layer" "$name" "exit $? — see log; rerun: $*"
  fi
}

# manual <layer> <name> <why> : a check a human must perform/sign off.
manual() { echo "→ [$1] $2 (MANUAL)"; record MANUAL "$1" "$2" "$3"; }

# skip_if_missing <tool> <layer> <name> <command...>
run_if_tool() {
  local tool="$1" layer="$2" name="$3"; shift 3
  if command -v "$tool" >/dev/null 2>&1; then
    run_check "$layer" "$name" "$@"
  else
    record SKIP "$layer" "$name" "$tool not installed — install it to enable this gate"
  fi
}

echo "=== Acceptance gate · ${VERSION} · ${DATE_UTC} ==="

# ── Always-on core (every version's gate inherits these) ─────────────────────
run_if_tool pnpm  "Frontend"   "Lint/type/format (pnpm check)"      pnpm check
run_if_tool pnpm  "Frontend"   "Component+unit tests (Vitest)"      pnpm test
run_if_tool cargo "Backend"    "Rust format"                        bash -c 'cd src-tauri && cargo fmt --all --check'
run_if_tool cargo "Backend"    "Rust clippy (warnings=errors)"      bash -c 'cd src-tauri && cargo clippy --all-targets -- -D warnings'
run_if_tool cargo "Backend"    "Rust tests"                         bash -c 'cd src-tauri && cargo test'

# Contract / type-drift gate (specta) — regenerate then assert no diff.
if command -v pnpm >/dev/null 2>&1; then
  if pnpm gen:types >/tmp/ag_out 2>&1 && git diff --quiet -- packages/shared/src/bindings.ts; then
    record PASS "Contract" "specta type-drift" "bindings.ts in sync"
  else
    record FAIL "Contract" "specta type-drift" "run: pnpm gen:types && commit bindings.ts"
  fi
else
  record SKIP "Contract" "specta type-drift" "pnpm not installed"
fi

# Coverage (TEST_PLAN §2: backend >=75%, safety-critical >=90%).
run_if_tool cargo-llvm-cov "Coverage" "Backend lines >= 75%" \
  bash -c 'cd src-tauri && cargo llvm-cov --summary-only --fail-under-lines 75'

# Security / supply chain.
run_if_tool cargo-deny "Security" "Supply chain (cargo deny)" \
  bash -c 'cd src-tauri && cargo deny check'
manual "Security" "No secret in logs (TC-P02)" "run a full E2E, scan logs for keys/tokens → zero hits"
manual "Privacy"  "No mail/cred to SeekerMail server (TC-P01)" "assert only the signed update check is outbound"

# Performance (8 P0 metrics @100k; owned by bench.yml).
if [[ -f bench-report.json ]]; then
  GATE="$(python3 -c 'import json;print(json.load(open("bench-report.json")).get("gate_result",""))' 2>/dev/null || true)"
  if [[ "$GATE" == "green" ]]; then
    record PASS "Performance" "8 P0 metrics @100k" "bench-report.json gate=green"
  else
    record FAIL "Performance" "8 P0 metrics @100k" "gate='$GATE' — rerun: cargo xtask bench --baseline bench-baseline.json"
  fi
else
  manual "Performance" "8 P0 metrics @100k" "run on Tier A: cargo xtask bench-seed --count 100000 && cargo xtask bench"
fi

# E2E happy path (real Tauri shell) — owned by nightly.yml.
manual "E2E" "Happy path: add → sync → search → send-to-sink (TC-A01/S01/C01/G01)" \
  "run nightly E2E green, or: pnpm test:e2e"

# Real-mailbox interop matrix (manual, pre-release).
manual "Interop" "Real-mailbox matrix Gmail/Outlook/IMAP (TEST_PLAN §3)" \
  "follow docs/quality/REAL_MAILBOX_RUNBOOK.md with disposable accounts"
manual "Send" "Real send proof on a real account (TC-G01 real-world)" \
  "pnpm tauri:dev (live-net), send one, record evidence in CHANGELOG Verified"

# ── Version-specific additions (TEST_PLAN.md §5) ─────────────────────────────
SMOKE=""
case "$VERSION" in
  v0.4)    SMOKE="scripts/smoke_v04.sh" ;;
  v0.5)    SMOKE="scripts/smoke_v05.sh"
           manual "AI" "BYO-AI provider contract + MockProvider draft suite (TC-F*/E*)" "run safety.yml + compliance.yml green"
           manual "Privacy" "Data-flow disclosure 100% (TC-F02)" "verify non-bypassable consent gate" ;;
  v0.6)    SMOKE="scripts/smoke_v06.sh"
           manual "AI" "Semi-auto draft suite + 'should not draft' >=90% (TC-E02/E03)" "run safety.yml green"
           manual "Journey" "User-journey surface suite §J (trust-ramp/quiet-hours/T4)" "run TC-J01..J20 green" ;;
  v0.7-rc) SMOKE="scripts/smoke_v07_rc.sh"
           run_if_tool cargo "AI-safety" "Full AI safety suite (mis-send<5%, fallback 10-30%)" \
             bash -c 'cd src-tauri && cargo xtask safety-run'
           manual "AI-safety" "Audit 7-day retrievable + CSV (TC-E0x)" "second reviewer re-runs safety suite + inspects audit"
           manual "Consent" "First-run consent non-bypassable" "verify cannot proceed without explicit consent" ;;
  v1.0)    SMOKE="scripts/smoke_v10_ga.sh"
           manual "Performance" "Attachment full-text + cross-account M9/M10; Windows M11" "run bench on Tier A + Tier C"
           manual "Security" "Third-party security audit, no Critical" "attach external audit report"
           manual "UI" "Pixel-parity check" "run parity_check.sh + visual diff" ;;
esac

if [[ -n "$SMOKE" && -x "$SMOKE" ]]; then
  run_check "Smoke" "Version smoke ($SMOKE)" "$SMOKE"
elif [[ -n "$SMOKE" ]]; then
  record SKIP "Smoke" "Version smoke ($SMOKE)" "script not found or not executable"
fi

# ── Render the one-page acceptance report ────────────────────────────────────
pass=0; fail=0; man=0; skip=0
for r in "${ROWS[@]}"; do case "${r%%|*}" in PASS) ((pass++));; FAIL) ((fail++));; MANUAL) ((man++));; SKIP) ((skip++));; esac; done

if [[ "$fail" -gt 0 ]]; then VERDICT="🔴 NO-GO — ${fail} failing check(s)"
elif [[ "$man" -gt 0 ]]; then VERDICT="🟡 CONDITIONAL — ${man} manual check(s) need human sign-off"
else VERDICT="🟢 GO — all automated checks green"; fi

{
  echo "# Acceptance Report · ${VERSION}"
  echo
  echo "| Field | Value |"
  echo "| --- | --- |"
  echo "| Version / 版本 | ${VERSION} |"
  echo "| Generated / 生成时间 | ${DATE_UTC} |"
  echo "| Commit | $(git rev-parse --short HEAD 2>/dev/null || echo n/a) |"
  echo "| Verdict / 结论 | **${VERDICT}** |"
  echo "| Summary / 汇总 | ${pass} PASS · ${fail} FAIL · ${man} MANUAL · ${skip} SKIP |"
  echo
  echo "> 自动检查全绿且所有 MANUAL 项已人工签字,方可放行。/ Release only when automated checks are green AND every MANUAL row is signed off."
  echo
  echo "## Checks"
  echo
  echo "| Status | Layer | Check | Detail |"
  echo "| --- | --- | --- | --- |"
  for r in "${ROWS[@]}"; do
    IFS='|' read -r st ly nm dt <<< "$r"
    case "$st" in PASS) icon="🟢 PASS";; FAIL) icon="🔴 FAIL";; MANUAL) icon="🟡 MANUAL";; SKIP) icon="⚪ SKIP";; esac
    echo "| ${icon} | ${ly} | ${nm} | ${dt} |"
  done
  echo
  echo "## Manual sign-off / 人工签字"
  echo
  echo "| Role / 角色 | Name / 姓名 | Date / 日期 | Signature / 签字 |"
  echo "| --- | --- | --- | --- |"
  echo "| Release engineer / 发版工程师 |  |  |  |"
  echo "| QA / 质量负责人 |  |  |  |"
  echo "| AI-safety reviewer (v0.7 RC+) / AI 安全复核 |  |  |  |"
  echo
  echo "_Generated by scripts/acceptance_gate.sh (TAS S4). See docs/quality/TEST_AND_ACCEPTANCE_SYSTEM_MANUAL.md ch.4._"
} > "$REPORT"

echo
echo "Verdict: ${VERDICT}"
echo "Report written: ${REPORT}"
echo "(${pass} PASS · ${fail} FAIL · ${man} MANUAL · ${skip} SKIP)"

exit "$FAILED"
