#!/usr/bin/env bash
# Verify an Authenticode signature on a Windows artifact (T115 §8). Non-zero exit
# on any unverified file. Called at the end of the release.yml Windows leg.
#
# Usage: scripts/verify_windows_sig.sh <file> [<file> ...]
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <file> [<file> ...]" >&2
  exit 2
fi

failed=0
for file in "$@"; do
  if signtool.exe verify /pa /v "$file"; then
    echo "  ✓ verified: $file"
  else
    echo "  ✗ signature verification FAILED: $file" >&2
    failed=1
  fi
done

[ "$failed" -eq 0 ] && echo "✓ All Windows signatures verified." || exit 1
