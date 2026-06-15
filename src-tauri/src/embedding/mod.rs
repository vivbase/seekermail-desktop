//! `embedding` — the local text → vector runtime (B3, 03 §10, ADR-0003).
//!
//! ## Backend note (deliberate, mirrors T019's `VectorStore`)
//!
//! The card (T030) specifies bundling the bge-m3 ONNX model (~2.2 GB) and running
//! it through ONNX Runtime (`ort`). To keep the **default** build dependency-light
//! and green on CI without a 2.2 GB model or a GPU — exactly as T019 shipped a
//! brute-force `VectorStore` behind the LanceDB API — the real ONNX backend lives
//! behind the OFF-by-default `local-embed` feature ([`onnx`]). The default build
//! wires a deterministic, dependency-free [`OfflineBackend`] that produces stable,
//! L2-normalised 1024-dim vectors with bag-of-words locality, so the whole B3
//! pipeline (chunk → embed → vector upsert → ANN search, T031/T033) is exercised
//! end-to-end in tests and dev without the model. Enable `--features local-embed`
//! (with the T010 model in `resources/`) for the real bge-m3 runtime.
//!
//! ## API shape (one documented deviation from T030 §3)
//!
//! T030 lists `embed(&str) -> AppResult<[f32; 1024]>`. We expose `Vec<f32>` of
//! length [`DIM`] instead: it interops directly with [`crate::vector::VectorRow`]
//! (whose `vector` field is `Vec<f32>`) and with the `local-embed` ONNX tensor
//! output, avoiding a const-generic array round-trip at every call site. The
//! invariant the acceptance criteria care about — length == 1024, L2 norm ≈ 1.0,
//! deterministic — is preserved and unit-tested.

pub mod chunker;

pub mod queue;

#[cfg(feature = "local-embed")]
pub mod onnx;

use std::sync::Arc;

use crate::config::Paths;
use crate::error::{AppError, AppResult};

/// bge-m3 output dimensionality (03 §10, ADR-0003). Must equal
/// [`crate::vector::schema::VECTOR_DIM`]; asserted by a unit test below.
pub const DIM: usize = 1024;

/// Model identifier stamped onto every vector row (`mails.embedding_model`,
/// `email_vectors.embedding_model`). The offline backend reports the same name so
/// downstream filtering by model is consistent across builds.
pub const MODEL_NAME: &str = "bge-m3";

/// A text → vector embedder. Cheap to `Clone` (the backend sits behind an `Arc`);
/// every background worker and command holds its own clone.
#[derive(Clone)]
pub struct Embedder {
    inner: Arc<Backend>,
}

enum Backend {
    /// Deterministic, dependency-free embedder (default build + tests).
    Offline(OfflineBackend),
    /// Real ONNX Runtime bge-m3 (feature `local-embed`).
    #[cfg(feature = "local-embed")]
    Onnx(onnx::OnnxBackend),
}

impl Embedder {
    /// Pick the best available backend without ever failing boot.
    ///
    /// * Default build → [`OfflineBackend`].
    /// * `local-embed` → real ONNX if the model + tokenizer exist in `resources/`
    ///   and the SHA-256 in `model.lock.json` matches; otherwise logs a warning and
    ///   falls back to the offline backend (the spec's "Recoverable — search index
    ///   needs rebuilding" stance, 09 §4 — never a hard boot crash).
    ///
    /// Callers that need to surface "real model unavailable" to the UI can inspect
    /// [`Embedder::is_offline`] after boot and emit `gte:error`.
    pub fn load(paths: &Paths) -> Self {
        #[cfg(feature = "local-embed")]
        {
            match onnx::OnnxBackend::load(paths) {
                Ok(backend) => {
                    tracing::info!(
                        event = "embedder_ready",
                        backend = "onnx",
                        model = MODEL_NAME
                    );
                    return Self {
                        inner: Arc::new(Backend::Onnx(backend)),
                    };
                }
                Err(e) => {
                    tracing::warn!(
                        event = "embedder_fallback",
                        error = %e,
                        "ONNX embedder unavailable; falling back to offline embedder"
                    );
                }
            }
        }
        #[cfg(not(feature = "local-embed"))]
        let _ = paths; // offline backend needs no files

        tracing::info!(
            event = "embedder_ready",
            backend = "offline",
            model = MODEL_NAME
        );
        Self {
            inner: Arc::new(Backend::Offline(OfflineBackend)),
        }
    }

    /// Construct the deterministic offline backend directly, bypassing model
    /// detection. Used by tests and by any pipeline that wants reproducible
    /// vectors regardless of the `local-embed` feature.
    pub fn offline() -> Self {
        Self {
            inner: Arc::new(Backend::Offline(OfflineBackend)),
        }
    }

    /// True when the deterministic offline backend is in use (no real model loaded).
    pub fn is_offline(&self) -> bool {
        matches!(&*self.inner, Backend::Offline(_))
    }

    /// Embed one string into a length-[`DIM`] L2-normalised vector.
    ///
    /// Pure and synchronous. Callers in an async context **must** wrap this in
    /// [`tokio::task::spawn_blocking`] (03 §15); [`Embedder::embed_blocking`] /
    /// [`Embedder::embed_batch_blocking`] do that for you.
    pub fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        match &*self.inner {
            Backend::Offline(b) => Ok(b.embed(text)),
            #[cfg(feature = "local-embed")]
            Backend::Onnx(b) => b.embed(text),
        }
    }

    /// Batch variant. The contract (T030 §7) is that `embed_batch(texts)[i]` is
    /// numerically identical to `embed(texts[i])`.
    pub fn embed_batch(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        match &*self.inner {
            Backend::Offline(b) => Ok(texts.iter().map(|t| b.embed(t)).collect()),
            #[cfg(feature = "local-embed")]
            Backend::Onnx(b) => b.embed_batch(texts),
        }
    }

    /// `embed` off the async runtime via `spawn_blocking` (03 §15). The embedder is
    /// cloned into the blocking task so no borrow escapes.
    pub async fn embed_blocking(&self, text: String) -> AppResult<Vec<f32>> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.embed(&text))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("embed task join: {e}")))?
    }

    /// `embed_batch` off the async runtime via `spawn_blocking` (03 §15).
    pub async fn embed_batch_blocking(&self, texts: Vec<String>) -> AppResult<Vec<Vec<f32>>> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.embed_batch(&texts))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("embed batch task join: {e}")))?
    }
}

impl std::fmt::Debug for Embedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &*self.inner {
            Backend::Offline(_) => "offline",
            #[cfg(feature = "local-embed")]
            Backend::Onnx(_) => "onnx",
        };
        write!(f, "Embedder {{ backend: {backend}, dim: {DIM} }}")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Offline backend — deterministic feature-hashing embedder
// ─────────────────────────────────────────────────────────────────────────────

/// A dependency-free, deterministic embedder. It hashes each word into the 1024-d
/// space (signed feature hashing, two probes per token to cut collisions), then
/// L2-normalises. Documents that share words land closer in cosine space, so the
/// brute-force ANN (T019/T033) returns sensible neighbours in dev and tests — while
/// the output is fully reproducible (a pure function of the input bytes).
struct OfflineBackend;

impl OfflineBackend {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; DIM];
        let mut any = false;
        for token in tokenize(text) {
            any = true;
            // Two independent probes (different seeds) per token.
            for seed in [0xcbf2_9ce4_8422_2325u64, 0x1000_0000_01b3u64] {
                let h = fnv1a64_seeded(token.as_bytes(), seed);
                let idx = (h % DIM as u64) as usize;
                let sign = if (h >> 63) & 1 == 1 { 1.0 } else { -1.0 };
                v[idx] += sign;
            }
        }
        if !any {
            // Empty / punctuation-only input: a deterministic unit vector keeps the
            // L2-norm ≈ 1.0 invariant (T031 skips empty bodies upstream anyway).
            v[0] = 1.0;
            return v;
        }
        l2_normalize(&mut v);
        v
    }
}

/// Lowercase word tokens (`\w+`-ish) without pulling in `regex` at call time.
fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
}

/// FNV-1a 64-bit with a tunable offset basis — small, fast, fully deterministic
/// across platforms (unlike `std::hash::DefaultHasher`, whose output is unstable).
fn fnv1a64_seeded(bytes: &[u8], offset_basis: u64) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = offset_basis;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// In-place L2 normalisation; a zero vector is left untouched.
fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SHA-256 model-integrity guard (used by the ONNX backend; tested standalone)
// ─────────────────────────────────────────────────────────────────────────────

/// The `resources/model.lock.json` written by T010 alongside the ONNX file.
#[derive(Debug, serde::Deserialize)]
pub struct ModelLock {
    /// Lower-case hex SHA-256 of the ONNX model file.
    pub sha256: String,
    /// Optional human label (e.g. "bge-m3-fp32"); informational only.
    #[serde(default)]
    pub model: String,
}

/// Verify a model file against its `model.lock.json`. Returns `Ok(())` on match,
/// [`AppError::GteCorrupt`] on mismatch (09 §4: Recoverable — "search index needs
/// rebuilding"), or an I/O error if either file is unreadable.
pub fn verify_model_checksum(
    model_path: &std::path::Path,
    lock_path: &std::path::Path,
) -> AppResult<()> {
    use sha2::{Digest, Sha256};

    let lock_bytes = std::fs::read(lock_path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("read model lock: {e}")))?;
    let lock: ModelLock = serde_json::from_slice(&lock_bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("parse model lock: {e}")))?;

    let model_bytes = std::fs::read(model_path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("read model file: {e}")))?;
    let digest = Sha256::digest(&model_bytes);
    let actual = hex_lower(&digest);

    if actual.eq_ignore_ascii_case(lock.sha256.trim()) {
        Ok(())
    } else {
        Err(AppError::GteCorrupt)
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    fn l2(v: &[f32]) -> f32 {
        v.iter().map(|x| x * x).sum::<f32>().sqrt()
    }

    #[test]
    fn dim_matches_vector_schema() {
        assert_eq!(
            DIM,
            crate::vector::schema::VECTOR_DIM,
            "embedding DIM must equal vector store dim"
        );
    }

    #[test]
    fn embed_is_unit_length_and_right_dim() {
        let e = Embedder {
            inner: Arc::new(Backend::Offline(OfflineBackend)),
        };
        let v = e.embed("Hello world").unwrap();
        assert_eq!(v.len(), DIM);
        assert!(
            approx(l2(&v), 1.0, 1e-3),
            "L2 norm should be ~1.0, was {}",
            l2(&v)
        );
    }

    #[test]
    fn embed_is_deterministic() {
        let e = Embedder {
            inner: Arc::new(Backend::Offline(OfflineBackend)),
        };
        let a = e.embed("the quarterly report is attached").unwrap();
        let b = e.embed("the quarterly report is attached").unwrap();
        assert_eq!(a, b, "same text must yield identical vectors");
    }

    #[test]
    fn batch_matches_single() {
        let e = Embedder {
            inner: Arc::new(Backend::Offline(OfflineBackend)),
        };
        let texts = vec!["text one".to_string(), "another text two".to_string()];
        let batch = e.embed_batch(&texts).unwrap();
        assert_eq!(batch.len(), 2);
        for (i, t) in texts.iter().enumerate() {
            let single = e.embed(t).unwrap();
            for (x, y) in batch[i].iter().zip(single.iter()) {
                assert!(approx(*x, *y, 1e-6), "batch[{i}] differs from single embed");
            }
        }
    }

    #[test]
    fn similar_text_is_closer_than_unrelated() {
        // Bag-of-words locality: shared words → higher cosine. Guards the offline
        // backend's usefulness for ANN, not a property of the real model.
        let e = Embedder {
            inner: Arc::new(Backend::Offline(OfflineBackend)),
        };
        let q = e.embed("invoice payment due next week").unwrap();
        let near = e.embed("payment for the invoice is due").unwrap();
        let far = e.embed("lunch plans for saturday afternoon").unwrap();
        let cos = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(
            cos(&q, &near) > cos(&q, &far),
            "related text must score higher"
        );
    }

    #[test]
    fn empty_text_is_safe_unit_vector() {
        let e = Embedder {
            inner: Arc::new(Backend::Offline(OfflineBackend)),
        };
        let v = e.embed("   ...  ").unwrap();
        assert_eq!(v.len(), DIM);
        assert!(approx(l2(&v), 1.0, 1e-3));
    }

    #[test]
    fn checksum_passes_then_fails_on_tamper() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        let lock = dir.path().join("model.lock.json");
        std::fs::write(&model, b"fake-onnx-bytes").unwrap();

        // Correct lock.
        let sha = {
            use sha2::{Digest, Sha256};
            hex_lower(&Sha256::digest(b"fake-onnx-bytes"))
        };
        let mut f = std::fs::File::create(&lock).unwrap();
        write!(f, r#"{{"sha256":"{sha}","model":"bge-m3-fp32"}}"#).unwrap();
        drop(f);
        assert!(verify_model_checksum(&model, &lock).is_ok());

        // Tamper the model → GteCorrupt.
        std::fs::write(&model, b"tampered-bytes").unwrap();
        assert!(matches!(
            verify_model_checksum(&model, &lock).unwrap_err(),
            AppError::GteCorrupt
        ));
    }
}
