#!/usr/bin/env node
// Dependency-free npm license gate.
//
// Walks node_modules and fails (exit 1) if any third-party package uses a
// license outside the allowlist - in particular any GPL/AGPL/LGPL/SSPL license,
// which is incompatible with shipping an open-core product with a proprietary
// Pro tier. This mirrors deny.toml on the Rust side.
//
// Run: node scripts/ci/check-licenses.mjs
import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = 'node_modules';

// Our own workspace packages - they are AGPL-3.0-only and must be skipped.
const SELF = new Set(['seekermail-desktop', '@shared']);

// Permissive + weak-copyleft (MPL) licenses we accept. Anything else fails.
const ALLOW = new Set([
  'MIT', 'MIT-0', 'ISC', '0BSD', 'BSD-2-Clause', 'BSD-3-Clause', 'BSD',
  'Apache-2.0', 'Apache 2.0', 'Zlib', 'Unlicense', 'CC0-1.0', 'CC-BY-4.0',
  'BlueOak-1.0.0', 'Python-2.0', 'OFL-1.1', 'WTFPL', 'MPL-2.0',
]);

// Evaluate a (possibly compound) SPDX expression: OR passes if any side passes;
// AND passes only if every side passes.
function spdxOk(expr) {
  if (!expr) return false;
  const e = expr.trim();
  if (/\bOR\b/i.test(e)) return e.split(/\bOR\b/i).some(spdxOk);
  if (/\bAND\b/i.test(e)) return e.split(/\bAND\b/i).every(spdxOk);
  return ALLOW.has(e.replace(/^[()\s]+|[()\s]+$/g, ''));
}

function licenseOf(pkg) {
  if (typeof pkg.license === 'string') return pkg.license;
  if (pkg.license && typeof pkg.license === 'object' && pkg.license.type) return pkg.license.type;
  if (Array.isArray(pkg.licenses)) return pkg.licenses.map((l) => (l && l.type) || l).join(' OR ');
  return '';
}

const seen = new Map(); // "name@version" -> license string
function walk(dir) {
  let entries;
  try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const ent of entries) {
    if (!ent.isDirectory()) continue; // skip symlinks (pnpm dep links) to avoid loops
    const p = join(dir, ent.name);
    const pj = join(p, 'package.json');
    if (existsSync(pj)) {
      try {
        const pkg = JSON.parse(readFileSync(pj, 'utf8'));
        if (pkg.name && pkg.version) seen.set(`${pkg.name}@${pkg.version}`, licenseOf(pkg));
      } catch { /* ignore unreadable package.json */ }
    }
    // Recurse into scopes (@scope/*), the pnpm virtual store (.pnpm), and nested node_modules.
    if (ent.name === '.pnpm' || ent.name.startsWith('@')) walk(p);
    walk(join(p, 'node_modules'));
  }
}
walk(ROOT);

const violations = [];
for (const [id, lic] of seen) {
  const name = id.slice(0, id.lastIndexOf('@'));
  if (SELF.has(name)) continue;
  if (!spdxOk(lic)) violations.push(`${id}  ->  ${lic || '(no license field)'}`);
}

if (violations.length) {
  console.error(`License gate FAILED - ${violations.length} package(s) outside the allowlist:`);
  for (const v of violations.sort()) console.error('  ' + v);
  console.error('\nIf a package is genuinely acceptable, add its SPDX id to ALLOW in scripts/ci/check-licenses.mjs.');
  console.error('A GPL/AGPL/LGPL/SSPL package here would force the whole app copyleft - investigate before allowing.');
  process.exit(1);
}
console.log(`License gate passed - ${seen.size} package(s) checked, all within the allowlist.`);
