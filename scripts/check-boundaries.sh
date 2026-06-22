#!/usr/bin/env bash
# Static boundary checks the type system can't express (07 §6, §7).
#  1. No bare hex colors in TS/TSX — components must use design tokens.
#  2. @tauri-apps/api may be imported ONLY under src/ipc/.
# Used by CI and the skeleton smoke gate. Exits non-zero on any violation.
set -uo pipefail
cd "$(dirname "$0")/.."

fail=0

echo "• checking for bare hex colors in src/ …"
# Components must use Seeker design tokens (07 §7). A small, audited set of
# sources legitimately carry literal hex and is exempt — keep this list tight:
#   • *.test.ts / *.test.tsx      — test fixtures, never shipped UI
#   • ComposeFormatBar.tsx        — rich-text colour picker; literal swatches ARE the feature
#   • routes/repository/data.ts   — mock data (invoice refs like "#4471", var() fallbacks)
#   • SVG stroke/fill/stop-color  — presentation attributes on inline icons
hex_hits="$(
  grep -RInE '#[0-9a-fA-F]{3,8}\b' src --include='*.ts' --include='*.tsx' \
    | grep -vE '\.test\.tsx?:' \
    | grep -vE '/ComposeFormatBar\.tsx:' \
    | grep -vE '/repository/data\.ts:' \
    | grep -vE '(stroke|fill|stop-color)="#[0-9a-fA-F]{3,8}"' \
    || true
)"
if [ -n "$hex_hits" ]; then
  echo "$hex_hits"
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
