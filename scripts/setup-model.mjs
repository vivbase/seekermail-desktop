#!/usr/bin/env node
// Idempotent ONNX embedding-model fetch (T010, 08 §1/§2/§8).
//
// Downloads + checksum-verifies the bge-m3 ONNX assets (BAAI/bge-m3, MIT) into
// `src-tauri/resources/`. Weights are large and git-ignored; the pinned
// `model.lock.json` (repo + commit revision + sha256 + bytes) IS checked in.
//
// Trust-on-first-use (08 §2): the FIRST run resolves the repo's `main` to an
// immutable commit SHA, downloads that revision, records the sha256, and writes
// `model.lock.json`. Every later run/CI/machine verifies against the lock and
// skips when the checksum matches — so the artifact is reproducible and the
// revision is never a moving `main` after the first pin. Override the revision
// explicitly with `SEEKERMAIL_MODEL_REVISION=<sha>`.
//
// No inference here (that's B3). The app never downloads models at runtime.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const MODEL_REPO = "BAAI/bge-m3";
// Assets pulled from the pinned revision. The dense ONNX export lives under onnx/.
// bge-m3 uses ONNX *external data*: `model.onnx` is just the graph (~700 KB) and the
// weights (~2.2 GB) live alongside it in `model.onnx_data`. BOTH are required — ort
// loads the data file by the basename referenced inside model.onnx, so it must sit
// next to model.onnx in resources/. (Fetching only the graph yields a model that
// fails to load at inference time.)
const ASSETS = [
  "onnx/model.onnx",
  "onnx/model.onnx_data",
  "tokenizer.json",
  "config.json",
];

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const RESOURCES_DIR = path.resolve(__dirname, "../src-tauri/resources");
const LOCK_PATH = path.join(RESOURCES_DIR, "model.lock.json");
const HF = "https://huggingface.co";

function die(msg) {
  console.error(`\n✗ ${msg}\n`);
  process.exit(1);
}

function assetUrl(revision, asset) {
  return `${HF}/${MODEL_REPO}/resolve/${revision}/${asset}`;
}

async function sha256File(file) {
  const hash = crypto.createHash("sha256");
  await pipeline(fs.createReadStream(file), hash);
  return hash.digest("hex");
}

// Resolve the repo's default branch to an immutable commit SHA (first run only).
async function resolveRevision() {
  const fromEnv = process.env.SEEKERMAIL_MODEL_REVISION;
  if (fromEnv && /^[0-9a-f]{7,40}$/i.test(fromEnv)) return fromEnv;
  const res = await fetch(`${HF}/api/models/${MODEL_REPO}`);
  if (!res.ok) {
    die(
      `couldn't resolve a commit SHA for ${MODEL_REPO} (HTTP ${res.status}).\n` +
        `  Pin one explicitly: SEEKERMAIL_MODEL_REVISION=<sha> pnpm run setup:model`,
    );
  }
  const meta = await res.json();
  if (!meta.sha) die(`HuggingFace metadata for ${MODEL_REPO} had no commit sha.`);
  return meta.sha;
}

// Stream a URL to disk (never buffer the whole file) while hashing + reporting.
async function downloadTo(url, dest) {
  const res = await fetch(url);
  if (!res.ok || !res.body) {
    die(
      `download failed: ${url} (HTTP ${res.status}).\n` +
        `  Check the model path/revision, then re-run: pnpm run setup:model`,
    );
  }
  const total = Number(res.headers.get("content-length") || 0);
  const hash = crypto.createHash("sha256");
  let seen = 0;
  let lastPct = -1;

  const source = Readable.fromWeb(res.body);
  source.on("data", (chunk) => {
    hash.update(chunk);
    seen += chunk.length;
    if (total) {
      const pct = Math.floor((seen / total) * 100);
      if (pct !== lastPct && pct % 5 === 0) {
        process.stdout.write(`\r  ${path.basename(dest)} ${pct}%   `);
        lastPct = pct;
      }
    }
  });

  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = `${dest}.part`;
  await pipeline(source, fs.createWriteStream(tmp));
  fs.renameSync(tmp, dest);
  process.stdout.write(`\r  ${path.basename(dest)} done            \n`);
  return { sha256: hash.digest("hex"), bytes: seen };
}

function primaryAssetPath() {
  return path.join(RESOURCES_DIR, path.basename(ASSETS[0])); // model.onnx
}

async function main() {
  fs.mkdirSync(RESOURCES_DIR, { recursive: true });
  const primary = primaryAssetPath();

  // ── Idempotent fast path: lock present + checksum matches → skip ──────────
  if (fs.existsSync(LOCK_PATH) && fs.existsSync(primary)) {
    const lock = JSON.parse(fs.readFileSync(LOCK_PATH, "utf8"));
    const actual = await sha256File(primary);
    if (actual === lock.sha256) {
      console.log(
        `✓ model already present (sha256 matches lock, rev ${lock.revision.slice(0, 12)})`,
      );
      return;
    }
    die(
      `model checksum mismatch for ${path.basename(primary)}.\n` +
        `  expected ${lock.sha256}\n  actual   ${actual}\n` +
        `  Delete the file and re-run: rm "${primary}" && pnpm run setup:model`,
    );
  }

  // ── First run: resolve + download + write the lock ───────────────────────
  const existingLock = fs.existsSync(LOCK_PATH)
    ? JSON.parse(fs.readFileSync(LOCK_PATH, "utf8"))
    : null;
  const revision = existingLock?.revision ?? (await resolveRevision());
  console.log(`Fetching ${MODEL_REPO} @ ${revision.slice(0, 12)} → src-tauri/resources/`);
  console.log("(bge-m3 fp32 is ~2.2 GB — this can take a while.)\n");

  let primaryInfo = null;
  for (const asset of ASSETS) {
    const dest = path.join(RESOURCES_DIR, path.basename(asset));
    const info = await downloadTo(assetUrl(revision, asset), dest);
    if (asset === ASSETS[0]) primaryInfo = info;
    // If the lock already pins this asset's sha256, enforce it.
    if (existingLock && asset === ASSETS[0] && existingLock.sha256 !== info.sha256) {
      die(`downloaded ${asset} sha256 ${info.sha256} ≠ locked ${existingLock.sha256}.`);
    }
  }

  if (!existingLock) {
    const lock = {
      repo: MODEL_REPO,
      revision,
      asset: ASSETS[0],
      sha256: primaryInfo.sha256,
      bytes: primaryInfo.bytes,
      pinnedAt: new Date().toISOString(),
    };
    fs.writeFileSync(LOCK_PATH, JSON.stringify(lock, null, 2) + "\n");
    console.log(
      `\n✓ wrote ${path.relative(process.cwd(), LOCK_PATH)} — commit it (weights stay git-ignored).`,
    );
  } else {
    console.log("\n✓ model fetched and verified against the existing lock.");
  }
}

main().catch((e) => die(e?.stack || String(e)));
