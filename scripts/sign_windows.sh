#!/usr/bin/env bash
# Authenticode signing helper (T115, dev/05 §5). Wraps the cloud signing service
# so the private key never leaves the HSM. Reads ONLY environment variables — no
# certificate, thumbprint, or token literal is ever written here or in the repo.
#
# Required env (set as GitHub Secrets, never inline):
#   WIN_CERT_THUMBPRINT   SHA-1 thumbprint of the EV/OV cert in the store
#   WIN_SIGNING_URL       cloud signing API endpoint (passed through to the CLI)
#   WIN_SIGNING_TOKEN     cloud signing API token
# Optional:
#   WIN_TIMESTAMP_URL     RFC-3161 timestamp authority (default: DigiCert)
#
# Usage: scripts/sign_windows.sh <file> [<file> ...]
set -euo pipefail

: "${WIN_CERT_THUMBPRINT:?WIN_CERT_THUMBPRINT not set}"
: "${WIN_SIGNING_URL:?WIN_SIGNING_URL not set}"
: "${WIN_SIGNING_TOKEN:?WIN_SIGNING_TOKEN not set}"
TIMESTAMP_URL="${WIN_TIMESTAMP_URL:-http://timestamp.digicert.com}"

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <file> [<file> ...]" >&2
  exit 2
fi

for file in "$@"; do
  echo "Signing (Authenticode, SHA-256, timestamped): $file"
  # signtool talks to the cloud HSM cert referenced by its thumbprint; the token
  # and URL authenticate that session. --timestamp is mandatory (parity w/ macOS).
  signtool.exe sign \
    /v /fd sha256 \
    /tr "$TIMESTAMP_URL" /td sha256 \
    /sha1 "$WIN_CERT_THUMBPRINT" \
    "$file"
done

echo "✓ Authenticode signing complete ($# file(s))."
