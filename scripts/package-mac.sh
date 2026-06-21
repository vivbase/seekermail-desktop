#!/usr/bin/env bash
#
# package-mac.sh — Build a release-mode SeekerMail .dmg for macOS (Apple Silicon).
#
# Run this ON YOUR MAC. It produces an UNSIGNED release dmg you can hand to a
# tester; see the Gatekeeper note printed at the end.
#
# Heads-up on size: the GTE embedding model (resources/model.onnx_data, ~2.2 GB)
# is bundled into the app, so the dmg is ~2 GB. The binary itself is ~12 MB — the
# model is the bulk. See README/notes if you want a slim build that downloads the
# model on first run instead.
#
# Usage:
#   bash scripts/package-mac.sh
#
set -euo pipefail

# Always run from the repo root, regardless of where the script is invoked.
cd "$(dirname "$0")/.."

APP="src-tauri/target/release/bundle/macos/SeekerMail.app"
MACOS_DIR="src-tauri/target/release/bundle/macos"
DMG_DIR="src-tauri/target/release/bundle/dmg"

echo "==> Toolchain"
echo "    node $(node -v)   pnpm $(pnpm -v)"
command -v cargo >/dev/null || { echo "ERROR: Rust/cargo not found on PATH"; exit 1; }

echo "==> Cleaning leftovers from any previous run"
# Tauri runs its bundle_dmg.sh helper FROM the macos/ bundle dir, so both the
# read-write temp images (rw.<pid>.*.dmg) AND the previous final dmg pile up
# there — not in dmg/. Tauri's bundled bundle_dmg.sh calls `hdiutil convert`
# WITHOUT the -ov flag, so a single leftover SeekerMail_*.dmg makes every later
# run abort with "hdiutil: convert failed - File exists". Detach any stale mounts
# (the helper mounts at /Volumes/dmg.XXXXXX) and delete the leftover images from
# both dirs so each build starts from a clean slate.
for vol in /Volumes/SeekerMail* /Volumes/dmg.*; do
  [ -d "$vol" ] && hdiutil detach "$vol" -force >/dev/null 2>&1 || true
done
rm -f "${MACOS_DIR}"/rw.*.dmg "${MACOS_DIR}"/SeekerMail*.dmg 2>/dev/null || true
rm -f "${DMG_DIR}"/rw.*.dmg   "${DMG_DIR}"/SeekerMail*.dmg   2>/dev/null || true

echo "==> Installing JS dependencies (frozen lockfile)"
pnpm install --frozen-lockfile

echo "==> Building release dmg"
echo "    (release-mode Rust compile — the first run can take 10-20 min)"
# Flag notes:
#   CI=true      -> makes Tauri pass --skip-jenkins to its dmg helper, which SKIPS
#                   the Finder window-styling AppleScript. That osascript step fails
#                   from a terminal/SSH session ("Not authorized to send Apple
#                   events") and was aborting the whole bundle. Skipping it yields a
#                   plain (un-prettified) but fully working dmg.
#   pnpm exec    -> call the tauri binary directly so flags reach tauri, not cargo
#                   (going through the tauri:build script inserts a literal `--`,
#                   which tauri forwards to cargo).
#   --features live-net -> ship the real IMAP/SMTP transports.
#   --bundles dmg / --config -> emit only the dmg, no updater artifact (so the build
#                   never needs a Tauri updater signing key).
if CI=true pnpm exec tauri build --features live-net --bundles dmg \
     --config '{"bundle":{"createUpdaterArtifacts":false}}'; then
  DMG="$(ls -t "${DMG_DIR}"/*.dmg | head -1)"
  echo ""
  echo "==> Done. Release dmg:"
  echo "    ${DMG}"
  echo "    size: $(du -h "${DMG}" | awk '{print $1}')"
  ARTIFACT="${DMG}"
else
  echo ""
  echo "==> dmg step failed — falling back to a zipped .app (same app, simpler wrapper)"
  [ -d "${APP}" ] || { echo "ERROR: ${APP} not found; the build did not produce an app"; exit 1; }
  ARTIFACT="src-tauri/target/release/bundle/SeekerMail-app.zip"
  rm -f "${ARTIFACT}"
  ditto -c -k --keepParent "${APP}" "${ARTIFACT}"
  echo "    ${ARTIFACT}"
  echo "    size: $(du -h "${ARTIFACT}" | awk '{print $1}')"
fi

echo ""
echo "==> ${ARTIFACT} is UNSIGNED. After a tester moves SeekerMail to /Applications,"
echo "    macOS will block the first launch. Tell them to run this once:"
echo ""
echo "        xattr -dr com.apple.quarantine /Applications/SeekerMail.app"
echo ""
echo "    (or right-click the app -> Open -> Open). Apple-Silicon Macs only."
