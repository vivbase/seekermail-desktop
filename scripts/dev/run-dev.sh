#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# SeekerMail dev launcher — NO macOS Keychain password prompt, ever.
#
# Why this exists: dev builds are not stably code-signed, so macOS re-asks for
# Keychain access on every launch. With SEEKERMAIL_DEV_VAULT set, *debug* builds
# keep credentials in a local JSON file instead of the OS Keychain, so there is
# nothing for macOS to prompt about. Release builds (`tauri build`) compile that
# path out and always use the real Keychain — this only affects development.
#
# You never type a password with this launcher. Use it for all dev runs.
#
# Usage:
#   scripts/dev/run-dev.sh           # offline build (default — no network)
#   scripts/dev/run-dev.sh --live    # live-net build (real IMAP/SMTP)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/../.."

VAULT_DIR="${SEEKERMAIL_DEV_VAULT_DIR:-$HOME/.seekermail}"
mkdir -p "$VAULT_DIR"
export SEEKERMAIL_DEV_VAULT="$VAULT_DIR/dev-vault.json"

# Bust the dev WebView cache so each launch loads the freshly-built frontend.
# The debug binary runs as "seekermail"; WKWebView caches its content-hashed
# assets keyed by the cached index.html, so a stale cache pins the old bundle.
# (Set SEEKERMAIL_KEEP_WEBCACHE=1 to skip this, e.g. for offline cache tests.)
if [[ "${SEEKERMAIL_KEEP_WEBCACHE:-0}" != "1" ]]; then
  for id in seekermail app.seekermail.desktop; do
    rm -rf "$HOME/Library/WebKit/$id" "$HOME/Library/Caches/$id" \
           "$HOME/Library/HTTPStorages/$id" "$HOME/Library/HTTPStorages/$id.binarycookies" 2>/dev/null || true
  done
fi

echo "▶ SeekerMail dev"
echo "  credential vault : $SEEKERMAIL_DEV_VAULT"
echo "  Keychain prompt  : disabled (debug dev-vault active)"

if [[ "${1:-}" == "--live" ]]; then
  echo "  network          : live-net (real IMAP/SMTP)"
  exec pnpm exec tauri dev --features live-net
else
  echo "  network          : offline"
  exec pnpm exec tauri dev
fi
