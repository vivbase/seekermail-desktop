#!/usr/bin/env bash
# cfg(target_os) audit (T116 §3e). Enumerates every conditional-compilation
# branch so a macOS-only path can't ship without a Windows counterpart (or an
# explicit, tracked TODO). Writes cfg_audit.txt and prints a summary. The human
# reconciles the list in the PR description; this script never fails the build.
set -uo pipefail
cd "$(dirname "$0")/.."

OUT="cfg_audit.txt"
grep -rn 'cfg(target_os' src-tauri/src/ >"$OUT" 2>/dev/null || true

macos=$(grep -c 'target_os = "macos"' "$OUT" 2>/dev/null || echo 0)
windows=$(grep -c 'target_os = "windows"' "$OUT" 2>/dev/null || echo 0)
other=$(grep -c 'not(any' "$OUT" 2>/dev/null || echo 0)

echo "cfg(target_os) branches → $OUT"
echo "  macos branches:        $macos"
echo "  windows branches:      $windows"
echo "  not(any(...)) stubs:   $other"
echo ""
echo "Review $OUT — every macOS branch should have a Windows peer or a tracked TODO:"
cat "$OUT"
