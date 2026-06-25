//! Real bge-m3 ONNX backend — compiled only under `--features local-embed`.
//!
//! This is the concrete runtime the T030 card describes: load the T010-fetched
//! `bge-m3` ONNX export through ONNX Runtime (`ort` 2.x), tokenize with the bundled
//! `tokenizer.json`, run inference, mean-pool the last hidden state under the
//! attention mask, and L2-normalise to a [`super::DIM`]-length vector.
//!
//! It sits behind the OFF-by-default `local-embed` feature so the default build and
//! CI stay free of the heavy `ort` + `tokenizers` native stack and the 2.2 GB model
//! (the same containment strategy T019 used for LanceDB). When the feature is off,
//! [`super::Embedder`] uses the deterministic offline backend instead.
//!
//! > NOTE: because the default delivery build does not enable `local-embed`, this
//! > module is not compiled or unit-tested in CI here; it is written against the
//! > `ort` 2.x / `tokenizers` 0.20 APIs and gated so enabling the feature wires the
//! > real runtime. Treat the first `--features local-embed` build as the
//! > integration checkpoint.

use std::path::Path;
use std::sync::{Arc, Mutex};

use ort::session::Session;
use ort::value::Value;
use tokenizers::Tokenizer;

use crate::config::Paths;
use crate::error::{AppError, AppResult};

use super::{verify_model_checksum, DIM};

/// bge-m3 context window (tokens). Chunks (T031) target ~400 tokens, well under it;
/// anything longer is truncated as a safety net.
const MAX_TOKENS: usize = 8192;

pub struct OnnxBackend {
    session: Mutex<Session>,
    tokenizer: Arc<Tokenizer>,
}

impl OnnxBackend {
    /// Load + checksum-verify the model, then build the ORT session. Returns
    /// [`AppError::GteCorrupt`] when `model.lock.json` doesn't match the model file.
    pub fn load(paths: &Paths) -> AppResult<Self> {
        let model_path = paths.model_onnx();
        let tokenizer_path = paths.model_tokenizer();
        let lock_path = paths.model_lock();

        if !model_path.exists() || !tokenizer_path.exists() {
            return Err(AppError::GteCorrupt);
        }
        verify_model_checksum(&model_path, &lock_path)?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("load tokenizer: {e}")))?;

        let session = build_session(&model_path)?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer: Arc::new(tokenizer),
        })
    }

    pub fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        Ok(self
            .run(&[text.to_string()])?
            .into_iter()
            .next()
            .unwrap_or_else(|| vec![0.0; DIM]))
    }

    pub fn embed_batch(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        self.run(texts)
    }

    /// Tokenise → pad to the batch max length → run → mean-pool → L2-normalise.
    fn run(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("tokenize: {e}")))?;

        let batch = encodings.len();
        let seq_len = encodings
            .iter()
            .map(|e| e.get_ids().len().min(MAX_TOKENS))
            .max()
            .unwrap_or(0)
            .max(1);

        let mut input_ids = vec![0i64; batch * seq_len];
        let mut attention = vec![0i64; batch * seq_len];
        for (b, enc) in encodings.iter().enumerate() {
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            for t in 0..ids.len().min(seq_len) {
                input_ids[b * seq_len + t] = ids[t] as i64;
                attention[b * seq_len + t] = mask[t] as i64;
            }
        }

        let ids_tensor = Value::from_array(([batch, seq_len], input_ids))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("ids tensor: {e}")))?;
        let mask_tensor = Value::from_array(([batch, seq_len], attention.clone()))
            .map_err(|e| AppError::Internal(anyhow::anyhow!("mask tensor: {e}")))?;

        let mut session = self
            .session
            .lock()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("session lock poisoned: {e}")))?;
        let outputs = session
            .run(ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
            ])
            .map_err(|e| AppError::Internal(anyhow::anyhow!("ort run: {e}")))?;

        // last_hidden_state: [batch, seq_len, hidden(=DIM)].
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("extract output: {e}")))?;
        let hidden = *shape.last().unwrap_or(&(DIM as i64)) as usize;
        if hidden != DIM {
            return Err(AppError::Internal(anyhow::anyhow!(
                "model hidden size {hidden} != expected {DIM}"
            )));
        }

        let mut out = Vec::with_capacity(batch);
        for b in 0..batch {
            let mut pooled = vec![0.0f32; DIM];
            let mut denom = 0.0f32;
            for t in 0..seq_len {
                let m = attention[b * seq_len + t] as f32;
                if m == 0.0 {
                    continue;
                }
                denom += m;
                let base = (b * seq_len + t) * hidden;
                for (d, p) in pooled.iter_mut().enumerate() {
                    *p += data[base + d] * m;
                }
            }
            if denom > 0.0 {
                for p in pooled.iter_mut() {
                    *p /= denom;
                }
            }
            super::l2_normalize(&mut pooled);
            out.push(pooled);
        }
        Ok(out)
    }
}

/// Build the ORT session. On macOS we try CoreML first (F_B3 §4.1 GPU target) and
/// fall back to CPU with a warn-log — never a hard failure (T030 §6).
fn build_session(model_path: &Path) -> AppResult<Session> {
    let mut builder = Session::builder()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("session builder: {e}")))?;

    #[cfg(target_os = "macos")]
    {
        use ort::execution_providers::CoreMLExecutionProvider;
        match builder
            .clone()
            .with_execution_providers([CoreMLExecutionProvider::default().build()])
        {
            Ok(b) => {
                tracing::info!(
                    event = "embedder_provider",
                    provider = "coreml",
                    "CoreML provider initialised"
                );
                builder = b;
            }
            Err(e) => {
                tracing::warn!(event = "embedder_provider", error = %e, "CoreML unavailable, using CPU");
            }
        }
    }

    builder
        .commit_from_file(model_path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("load onnx model: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Paths` rooted at a fresh temp dir with no bundled model files.
    fn modelless_paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let paths = Paths {
            db: root.join("seekermail.db"),
            vectors: root.join("vectors"),
            attachments: root.join("attachments"),
            logs: root.join("logs"),
            models: root.join("models"),
            resources: root.join("models"),
            root,
        };
        (tmp, paths)
    }

    /// With no `model.onnx` / `tokenizer.json` present, loading the real backend
    /// returns a clean, recoverable error (never a panic) — which `Embedder::load`
    /// turns into the offline fallback. This runs in the feature-build CI lane and
    /// needs no 2.2 GB model.
    #[test]
    fn load_without_model_files_is_gte_corrupt() {
        // `matches!` (not `unwrap_err`) so the test needs no `Debug` on the Ok
        // backend, which wraps a non-Debug ORT `Session`.
        let (_tmp, paths) = modelless_paths();
        assert!(matches!(
            OnnxBackend::load(&paths),
            Err(AppError::GteCorrupt)
        ));
    }
}
