#!/usr/bin/env node
// Dependency-free generator for JavaScript/npm third-party notices.
// Emits Markdown to stdout; invoked by scripts/ci/generate-notice.sh.
import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = 'node_modules';
const SELF = new Set(['seekermail-desktop', '@shared']);
const LICENSE_FILES = ['LICENSE', 'LICENSE.md', 'LICENSE.txt', 'LICENCE', 'LICENCE.md', 'COPYING', 'COPYING.md'];

function licenseOf(pkg) {
  if (typeof pkg.license === 'string') return pkg.license;
  if (pkg.license && pkg.license.type) return pkg.license.type;
  if (Array.isArray(pkg.licenses)) return pkg.licenses.map((l) => (l && l.type) || l).join(' OR ');
  return '(unspecified)';
}
function repoOf(pkg) {
  const r = pkg.repository;
  const url = pkg.homepage || (r && (r.url || r)) || '';
  return String(url).replace(/^git\+/, '').replace(/\.git$/, '');
}

const pkgs = new Map(); // "name@version" -> { license, dir, repo }
function walk(dir) {
  let entries;
  try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const ent of entries) {
    if (!ent.isDirectory()) continue;
    const p = join(dir, ent.name);
    const pj = join(p, 'package.json');
    if (existsSync(pj)) {
      try {
        const m = JSON.parse(readFileSync(pj, 'utf8'));
        if (m.name && m.version && !SELF.has(m.name)) {
          pkgs.set(`${m.name}@${m.version}`, { license: licenseOf(m), dir: p, repo: repoOf(m) });
        }
      } catch { /* ignore */ }
    }
    if (ent.name === '.pnpm' || ent.name.startsWith('@')) walk(p);
    walk(join(p, 'node_modules'));
  }
}
walk(ROOT);

function textOf(dir) {
  for (const f of LICENSE_FILES) {
    const fp = join(dir, f);
    if (existsSync(fp)) { try { return readFileSync(fp, 'utf8').trim(); } catch { /* ignore */ } }
  }
  return '';
}

const ids = [...pkgs.keys()].sort();
let out = `## JavaScript / npm dependencies\n\n`;
out += `This product bundles ${ids.length} npm packages. Their licenses (and, where shipped, full license texts) are reproduced below.\n\n`;
for (const id of ids) {
  const { license, dir, repo } = pkgs.get(id);
  out += `### ${id}\n\n- License: ${license}\n`;
  if (repo) out += `- Source: ${repo}\n`;
  const t = textOf(dir);
  if (t) out += `\n\`\`\`\n${t}\n\`\`\`\n`;
  out += '\n';
}
process.stdout.write(out);
