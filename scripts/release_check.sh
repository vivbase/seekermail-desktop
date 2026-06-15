#!/usr/bin/env bash
# Pre-tag release checklist (T056 §3, dev/05 §3.3). Run MANUALLY by the
# release engineer before pushing a v* tag — it is intentionally not wired
# into CI. Any failed check exits non-zero; fix and re-run until green.
#
# Usage: scripts/release_check.sh <tag>
#   e.g. scripts/release_check.sh v0.4.0-beta

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

TAG="${1:-}"
FAILED=0

fail() {
  echo "✗ $1" >&2
  FAILED=1
}

note() {
  echo "  $1"
}

# ── 1. Tag format ─────────────────────────────────────────────────────────────
echo "[1/4] Tag format"
if [[ -z "$TAG" ]]; then
  fail "no tag supplied — usage: scripts/release_check.sh v0.4.0-beta"
elif [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-(beta|rc|internal))?$ ]]; then
  note "tag '$TAG' matches the version scheme"
else
  fail "tag '$TAG' does not match v<MAJOR>.<MINOR>.<PATCH>[-beta|-rc|-internal]"
fi

# ── 2. CHANGELOG finalized ────────────────────────────────────────────────────
echo "[2/4] CHANGELOG finalized"
if grep -q "^## \[Unreleased\]" CHANGELOG.md; then
  # An empty Unreleased section is fine; content under it is not.
  UNRELEASED_BODY=$(awk '/^## \[Unreleased\]/{flag=1; next} /^## /{flag=0} flag' CHANGELOG.md | grep -cv '^\s*$' || true)
  if [[ "$UNRELEASED_BODY" -gt 0 ]]; then
    fail "CHANGELOG.md still has content under [Unreleased] — rename it to the release version first"
  else
    note "[Unreleased] section is empty"
  fi
else
  note "no [Unreleased] section"
fi

# ── 3. Performance gate (T055 artifact) ──────────────────────────────────────
echo "[3/4] Performance gate"
if [[ -f bench-report.json ]]; then
  GATE=$(python3 -c 'import json,sys; print(json.load(open("bench-report.json")).get("gate_result",""))' 2>/dev/null || true)
  if [[ "$GATE" == "green" ]]; then
    note "bench-report.json gate_result = green"
  else
    fail "bench-report.json gate_result = '$GATE' (need green; rerun: cargo xtask bench --baseline bench-baseline.json)"
  fi
else
  fail "bench-report.json not found — run: cargo xtask bench-seed && cargo xtask bench --baseline bench-baseline.json"
fi

# ── 4. Supply chain ───────────────────────────────────────────────────────────
echo "[4/4] Supply chain (cargo deny)"
if command -v cargo-deny >/dev/null 2>&1 || cargo deny --version >/dev/null 2>&1; then
  if (cd src-tauri && cargo deny check >/dev/null 2>&1); then
    note "cargo deny check passed"
  else
    fail "cargo deny check failed — see: cd src-tauri && cargo deny check"
  fi
else
  fail "cargo-deny is not installed — cargo install cargo-deny --locked"
fi

# ── 5. Windows signing readiness (T115) ──────────────────────────────────────
# Informational on a macOS-only release; enforced when REQUIRE_WINDOWS=1 (i.e. a
# GA build that must publish the windows-x86_64 updater entry).
echo "[5] Windows signing readiness (T115)"
if [[ -n "${WIN_CERT_THUMBPRINT:-}" ]]; then
  note "WIN_CERT_THUMBPRINT is set (Authenticode cert available)"
else
  if [[ "${REQUIRE_WINDOWS:-0}" == "1" ]]; then
    fail "WIN_CERT_THUMBPRINT is empty — required for a Windows-inclusive GA release"
  else
    note "WIN_CERT_THUMBPRINT not set — Windows leg will be skipped (set it + REQUIRE_WINDOWS=1 for GA)"
  fi
fi
if [[ -f scripts/sign_windows.sh && -f scripts/verify_windows_sig.sh ]]; then
  note "windows signing helpers present"
else
  fail "scripts/sign_windows.sh or verify_windows_sig.sh missing"
fi

echo
if [[ "$FAILED" -ne 0 ]]; then
  echo "✗ Pre-tag release check FAILED — fix the items above before tagging." >&2
  exit 1
fi
echo "✓ Pre-tag release check passed."
