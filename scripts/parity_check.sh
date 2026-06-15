#!/usr/bin/env bash
# Cross-platform parity — automated portion (T117 §3a/§6). Runs the path +
# keychain unit tests on the CURRENT OS; the screenshot/manual dimensions
# (OS notifications, RTL layout, CJK/Arabic fonts) are produced by the Release
# Engineer and recorded in docs/releases/v1.0.0_parity_report.md.
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0
ok() { echo "  ✓ $1"; }
bad() {
  echo "  ✗ $1"
  fail=1
}

echo "Cross-platform parity checks (T117)"

if command -v cargo >/dev/null; then
  (cd src-tauri && cargo test --lib -- config::tests >/tmp/parity_paths.log 2>&1) &&
    ok "Paths::resolve per-OS data dir test" || bad "Paths test (see /tmp/parity_paths.log)"
  (cd src-tauri && cargo test --lib -- keychain::tests >/tmp/parity_kc.log 2>&1) &&
    ok "keychain unit tests (item key + secret redaction)" || bad "keychain tests (see /tmp/parity_kc.log)"
else
  bad "cargo not available — cannot run path/keychain parity tests"
fi

echo "Manual parity items — capture screenshots into docs/releases/ and record PASS:"
echo "  • OS notification (macOS + Windows)"
echo "  • RTL layout (Arabic) on each OS"
echo "  • CJK + Arabic font rendering on each OS"

echo ""
if [ "$fail" -eq 0 ]; then
  echo "✓ parity automated checks passed"
else
  echo "✗ parity automated checks FAILED"
  exit 1
fi
