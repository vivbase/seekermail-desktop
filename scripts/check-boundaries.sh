#!/usr/bin/env bash
# Static boundary checks the type system can't express (07 §6, §7).
#  1. No bare hex colors in TS/TSX — components must use design tokens.
#  2. @tauri-apps/api may be imported ONLY under src/ipc/.
# Used by CI and the skeleton smoke gate. Exits non-zero on any violation.
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0

echo "• checking for bare hex colors in src/ …"
if grep -RInE '#[0-9a-fA-F]{3,8}\b' src --include='*.ts' --include='*.tsx'; then
  echo "  ✗ bare hex color found — use Seeker design tokens (07 §7)"
  fail=1
else
  echo "  ✓ none"
fi

echo "• checking @tauri-apps/api is only imported under src/ipc/ …"
hits="$(grep -RInE "from ['\"]@tauri-apps/api" src --include='*.ts' --include='*.tsx' | grep -v '^src/ipc/' || true)"
if [ -n "$hits" ]; then
  echo "$hits"
  echo "  ✗ @tauri-apps/api imported outside src/ipc/ (07 §6)"
  fail=1
else
  echo "  ✓ ok"
fi

if [ "$fail" -ne 0 ]; then
  echo "boundary checks FAILED"
  exit 1
fi
echo "boundary checks passed"
