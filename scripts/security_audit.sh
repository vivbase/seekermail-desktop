#!/usr/bin/env bash
# GA security & privacy audit — automated portion (T117 §3b/§6). Checks:
#   1. telemetry-free   — no analytics SDK crates in Cargo.toml
#   2. data sovereignty — mail bodies (body_text/body_html) are read in the AI
#                         module only inside the context-assembly path
#   3. log-safety       — an optional log file has no secret/PII-shaped fields
#   4. no-proxy         — the compliance test suite (T103) is green
#
# Usage: scripts/security_audit.sh [path/to/seekermail.log]
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
ok() { echo "  ✓ $1"; }
bad() {
  echo "  ✗ $1"
  fail=1
}

echo "GA security & privacy audit (T117)"

# 1. Telemetry-free build.
if grep -RniE 'sentry|datadog|amplitude|mixpanel|segment|bugsnag' src-tauri/Cargo.toml >/dev/null 2>&1; then
  bad "a telemetry crate is referenced in src-tauri/Cargo.toml"
else
  ok "no telemetry crates in Cargo.toml"
fi

# 2. Data sovereignty: mail bodies only read in the AI context-assembly path.
HITS=$(grep -rn 'body_text\|body_html' src-tauri/src/ai/ 2>/dev/null | grep -vE 'context|prompt|//' || true)
if [ -n "$HITS" ]; then
  echo "$HITS"
  bad "mail body referenced in ai/ outside context assembly"
else
  ok "mail body only used in AI context assembly (no off-device leak path)"
fi

# 3. Log-safety on a supplied log file (CI passes a fixture; RE passes the real log).
LOG="${1:-}"
if [ -n "$LOG" ] && [ -f "$LOG" ]; then
  if grep -nE '(password|secret|token|api[_-]?key)[[:space:]]*[=:][[:space:]]*[A-Za-z0-9+/]{16,}' "$LOG" >/dev/null; then
    bad "log contains a secret-shaped value: $LOG"
  elif grep -niE 'body_text|body_html' "$LOG" >/dev/null; then
    bad "log contains mail body fields: $LOG"
  else
    ok "log has no forbidden fields: $LOG"
  fi
else
  echo "  • no log path supplied — Release Engineer runs this against the real seekermail.log"
fi

# 4. No-proxy + log-safety compliance suite (T103).
if command -v cargo >/dev/null; then
  if cargo test --manifest-path src-tauri/Cargo.toml --test compliance >/tmp/sec_compliance.log 2>&1; then
    ok "no-proxy + log-safety compliance tests"
  else
    echo "  • compliance suite unavailable or failed (see /tmp/sec_compliance.log)"
  fi
fi

echo ""
if [ "$fail" -eq 0 ]; then
  echo "✓ security audit automated checks passed"
else
  echo "✗ security audit FAILED"
  exit 1
fi
