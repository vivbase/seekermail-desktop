#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Headless frontend verification — screenshot the LIVE vite dev server.
#
# Why this exists: a built `.app` bundle compiles the frontend INTO the binary,
# so it can never reflect source edits until a full `tauri build` re-embeds it.
# During development the truth is the vite dev server (http://localhost:1420),
# which `tauri dev` already loads with hot-module reload. This script renders any
# route from that server with headless Chromium and writes a PNG — no Tauri
# rebuild, no visible window, no Keychain/password prompt.
#
# The whole app renders here because the IPC layer (src/ipc/client.ts) falls back
# to MOCK_RESPONSES when `__TAURI_INTERNALS__` is absent (i.e. a plain browser).
#
# Setup is automatic and idempotent: Playwright + Chromium are installed once
# into ~/.seekermail-verify (user space — never needs sudo).
#
# Usage:
#   scripts/dev/verify-shots.sh "/"                 # dashboard
#   scripts/dev/verify-shots.sh "/gte,/repository"  # several routes at once
#
# Output: .verify-shots/<route>.png (gitignored). Console/page errors for each
# route are printed to stdout so runtime failures surface immediately.
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail
cd "$(dirname "$0")/../.."
REPO="$(pwd)"
ROUTES="${1:-/}"
VHOME="$HOME/.seekermail-verify"
mkdir -p "$VHOME"

if [[ ! -d "$VHOME/node_modules/playwright" ]]; then
  echo "▶ first-run setup: installing Playwright + Chromium into $VHOME"
  ( cd "$VHOME" \
    && { [[ -f package.json ]] || npm init -y >/dev/null 2>&1; } \
    && npm i playwright >/dev/null 2>&1 \
    && npx playwright install chromium >/dev/null 2>&1 )
fi

cat > "$VHOME/shot.mjs" <<'MJS'
import { chromium } from 'playwright';
import { mkdirSync } from 'fs';
const base = process.env.BASE || 'http://localhost:1420';
const routes = (process.argv[2] || '/').split(',');
const outdir = process.argv[3] || '.verify-shots';
mkdirSync(outdir, { recursive: true });
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1280, height: 832 }, deviceScaleFactor: 2 });
for (const route of routes) {
  const page = await ctx.newPage();
  const errs = [];
  page.on('console', (m) => { if (m.type() === 'error') errs.push(m.text()); });
  page.on('pageerror', (e) => errs.push('PAGEERROR: ' + e.message));
  const name = route.replace(/[^a-z0-9]+/gi, '_').replace(/^_|_$/g, '') || 'root';
  try {
    await page.goto(base + route, { waitUntil: 'networkidle', timeout: 30000 });
    await page.waitForTimeout(700);
    const f = outdir + '/' + name + '.png';
    await page.screenshot({ path: f });
    console.log('SHOT_OK ' + route + ' -> ' + f);
    if (errs.length) console.log('ERRORS[' + route + ']: ' + errs.slice(0, 8).join(' || '));
  } catch (e) {
    console.log('SHOT_FAIL ' + route + ': ' + e.message);
    if (errs.length) console.log('ERRORS[' + route + ']: ' + errs.slice(0, 8).join(' || '));
  }
  await page.close();
}
await browser.close();
console.log('ALL_DONE');
MJS

exec node "$VHOME/shot.mjs" "$ROUTES" "$REPO/.verify-shots"
