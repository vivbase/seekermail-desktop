//! `VectorStore` — the derived semantic index (T019, A3).
//!
//! ## Backend note (deliberate v0.2 deviation)
//! The card specifies an embedded **LanceDB** table. To keep the default build
//! free of the heavy `lancedb` + `arrow` native stack (and green on CI without a
//! verified vector toolchain), v0.2 ships a self-contained **brute-force cosine**
//! backend persisted to a JSON snapshot under `vectors/`. It implements the exact
//! [`VectorStore`] surface the downstream cards (B3 vectorize, C2 search) call —
//! `open / upsert / ann / delete_account / rebuild / stats` — so swapping in a
//! LanceDB backend later is a drop-in behind this type. The "SQLite authoritative,
//! index rebuildable from `embedding_status='indexed'`" invariant (01 Overview)
//! is preserved: the snapshot is purely derived and can be regenerated.

pub mod schema;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::util::now_unix;
use schema::VECTOR_DIM;

/// One chunk's row (one per `chunk_index` of a mail). `vector` length must equal
/// [`schema::VECTOR_DIM`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorRow {
    pub chunk_id: String,
    pub mail_id: String,
    pub chunk_index: i32,
    pub account_id: String,
    pub from_email: String,
    pub date_sent: i64,
    pub subject: String,
    pub snippet: String,
    pub embedding_model: String,
    pub vector: Vec<f32>,
}

/// A unit of work for a full rebuild (B3 will produce these from indexed mails).
pub type EmbeddingJob = VectorRow;

/// Metadata filter applied before/with the ANN scan (01 §Two-stage query).
#[derive(Debug, Clone, Default)]
pub struct AnnFilter {
    pub account_id: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
}

/// One ANN result.
#[derive(Debug, Clone)]
pub struct Hit {
    pub chunk_id: String,
    pub mail_id: String,
    pub score: f32,
}

/// Index statistics for `get_gte_status`.
#[derive(Debug, Clone, Copy)]
pub struct VectorStats {
    pub total_vectors: usize,
    pub last_rebuild_at: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
struct Snapshot {
    rows: Vec<VectorRow>,
    last_rebuild_at: Option<i64>,
}

struct Inner {
    rows: Vec<VectorRow>,
    snapshot_path: PathBuf,
    new_since_build: usize,
    last_rebuild_at: Option<i64>,
}

/// Embedded vector index. Cheap to share via `Arc`; all state behind one mutex.
pub struct VectorStore {
    inner: Mutex<Inner>,
}

impl VectorStore {
    /// Open (or create) the index rooted at `dir` (the app's `vectors/` path).
    pub fn open(dir: &Path) -> AppResult<Self> {
        std::fs::create_dir_all(dir)
            .map_err(|e| AppError::FsPermission(format!("create vectors dir: {e}")))?;
        let snapshot_path = dir.join("email_vectors.json");
        let snap = if snapshot_path.exists() {
            let bytes = std::fs::read(&snapshot_path)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("read vector snapshot: {e}")))?;
            serde_json::from_slice::<Snapshot>(&bytes).unwrap_or_default()
        } else {
            Snapshot::default()
        };
        Ok(Self {
            inner: Mutex::new(Inner {
                rows: snap.rows,
                snapshot_path,
                new_since_build: 0,
                last_rebuild_at: snap.last_rebuild_at,
            }),
        })
    }

    /// Batch upsert. LanceDB has no row-level upsert, so the contract (01) is
    /// "delete every chunk of a mail, then insert the new chunks" — replicated
    /// here per `mail_id`.
    pub fn upsert(&self, rows: &[VectorRow]) -> AppResult<()> {
        for r in rows {
            if r.vector.len() != VECTOR_DIM {
                return Err(AppError::Validation(format!(
                    "vector dim {} != {VECTOR_DIM}",
                    r.vector.len()
                )));
            }
        }
        let mut g = self.lock();
        let touched: std::collections::HashSet<&str> =
            rows.iter().map(|r| r.mail_id.as_str()).collect();
        // Replace only the mail's *body* chunks (chunk_id == "{mail_id}:{i}"). An
        // attachment's chunks share the same `mail_id` but live in a separate
        // chunk-id namespace ("{attachment_id}:{i}", T109), so a mail re-embed
        // must not wipe them — match on the chunk-id prefix, not `mail_id` alone.
        g.rows.retain(|existing| {
            !(touched.contains(existing.mail_id.as_str())
                && existing
                    .chunk_id
                    .starts_with(&format!("{}:", existing.mail_id)))
        });
        g.rows.extend(rows.iter().cloned());
        g.new_since_build += rows.len();
        let should_rebuild = g.should_rebuild();
        Self::persist(&g)?;
        if should_rebuild {
            // The brute-force backend has no index to rebuild; we simply reset the
            // counter and stamp the time so `stats()` reflects a "rebuild".
            g.new_since_build = 0;
            g.last_rebuild_at = Some(now_unix());
        }
        Ok(())
    }

    /// Upsert one attachment's chunks (T109). Replaces only rows whose `chunk_id`
    /// is prefixed `"{attachment_id}:"`, leaving the owning mail's body chunks and
    /// every other attachment's chunks untouched.
    pub fn upsert_attachment_chunks(
        &self,
        attachment_id: &str,
        rows: &[VectorRow],
    ) -> AppResult<()> {
        for r in rows {
            if r.vector.len() != VECTOR_DIM {
                return Err(AppError::Validation(format!(
                    "vector dim {} != {VECTOR_DIM}",
                    r.vector.len()
                )));
            }
        }
        let prefix = format!("{attachment_id}:");
        let mut g = self.lock();
        g.rows
            .retain(|existing| !existing.chunk_id.starts_with(&prefix));
        g.rows.extend(rows.iter().cloned());
        g.new_since_build += rows.len();
        Self::persist(&g)
    }

    /// Remove every chunk of one attachment (chunk_id prefix `"{attachment_id}:"`).
    pub fn delete_attachment_chunks(&self, attachment_id: &str) -> AppResult<()> {
        let prefix = format!("{attachment_id}:");
        let mut g = self.lock();
        g.rows.retain(|r| !r.chunk_id.starts_with(&prefix));
        Self::persist(&g)
    }

    /// k-nearest neighbours by cosine similarity, honouring the metadata filter.
    pub fn ann(&self, query: &[f32], k: usize, filter: AnnFilter) -> AppResult<Vec<Hit>> {
        if query.len() != VECTOR_DIM {
            return Err(AppError::Validation(format!(
                "query dim {} != {VECTOR_DIM}",
                query.len()
            )));
        }
        let g = self.lock();
        let mut hits: Vec<Hit> = g
            .rows
            .iter()
            .filter(|r| filter.matches(r))
            .map(|r| Hit {
                chunk_id: r.chunk_id.clone(),
                mail_id: r.mail_id.clone(),
                score: cosine(query, &r.vector),
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k);
        Ok(hits)
    }

    /// Remove every vector belonging to an account (account deletion).
    pub fn delete_account(&self, account_id: &str) -> AppResult<()> {
        let mut g = self.lock();
        g.rows.retain(|r| r.account_id != account_id);
        Self::persist(&g)
    }

    /// Full rebuild from a fresh stream of jobs (`start_reindex`).
    pub fn rebuild(&self, jobs: impl Iterator<Item = EmbeddingJob>) -> AppResult<VectorStats> {
        let mut g = self.lock();
        g.rows = jobs.collect();
        g.new_since_build = 0;
        g.last_rebuild_at = Some(now_unix());
        Self::persist(&g)?;
        Ok(VectorStats {
            total_vectors: g.rows.len(),
            last_rebuild_at: g.last_rebuild_at,
        })
    }

    pub fn stats(&self) -> AppResult<VectorStats> {
        let g = self.lock();
        Ok(VectorStats {
            total_vectors: g.rows.len(),
            last_rebuild_at: g.last_rebuild_at,
        })
    }

    /// Does at least one chunk exist for this mail? (T053 reindex verification.)
    pub fn contains_mail(&self, mail_id: &str) -> bool {
        let g = self.lock();
        g.rows.iter().any(|r| r.mail_id == mail_id)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("vector store mutex poisoned")
    }

    fn persist(inner: &Inner) -> AppResult<()> {
        let snap = Snapshot {
            rows: inner.rows.clone(),
            last_rebuild_at: inner.last_rebuild_at,
        };
        let bytes = serde_json::to_vec(&snap)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize vectors: {e}")))?;
        // Atomic write: temp file + rename.
        let tmp = inner.snapshot_path.with_extension("json.part");
        std::fs::write(&tmp, &bytes)
            .map_err(|e| AppError::FsPermission(format!("write vector snapshot: {e}")))?;
        std::fs::rename(&tmp, &inner.snapshot_path)
            .map_err(|e| AppError::FsPermission(format!("commit vector snapshot: {e}")))?;
        Ok(())
    }
}

impl Inner {
    fn should_rebuild(&self) -> bool {
        let total = self.rows.len();
        total > 0 && (self.new_since_build as f32 / total as f32) > schema::REBUILD_THRESHOLD
    }
}

impl AnnFilter {
    fn matches(&self, r: &VectorRow) -> bool {
        if let Some(acc) = &self.account_id {
            if &r.account_id != acc {
                return false;
            }
        }
        if let Some(from) = self.date_from {
            if r.date_sent < from {
                return false;
            }
        }
        if let Some(to) = self.date_to {
            if r.date_sent > to {
                return false;
            }
        }
        true
    }
}

/// Cosine similarity in `[-1, 1]`; 0 when either vector has zero norm.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, acc: &str, v: f32) -> VectorRow {
        VectorRow {
            chunk_id: format!("{id}:0"),
            mail_id: id.into(),
            chunk_index: 0,
            account_id: acc.into(),
            from_email: "a@x.com".into(),
            date_sent: 1000,
            subject: "s".into(),
            snippet: "sn".into(),
            embedding_model: "bge-m3".into(),
            vector: vec![v; VECTOR_DIM],
        }
    }

    #[test]
    fn open_upsert_ann_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path()).unwrap();
        store
            .upsert(&[row("m1", "acc", 1.0), row("m2", "acc", 0.1)])
            .unwrap();
        assert_eq!(store.stats().unwrap().total_vectors, 2);

        let q = vec![1.0f32; VECTOR_DIM];
        let hits = store.ann(&q, 1, AnnFilter::default()).unwrap();
        assert_eq!(hits[0].mail_id, "m1");
        assert!(hits[0].score > 0.99);

        store.delete_account("acc").unwrap();
        assert_eq!(store.stats().unwrap().total_vectors, 0);
    }

    #[test]
    fn open_is_idempotent_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = VectorStore::open(dir.path()).unwrap();
            store.upsert(&[row("m1", "acc", 1.0)]).unwrap();
        }
        // Re-open reads the persisted snapshot.
        let store = VectorStore::open(dir.path()).unwrap();
        assert_eq!(store.stats().unwrap().total_vectors, 1);
    }

    #[test]
    fn upsert_replaces_chunks_of_same_mail() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path()).unwrap();
        store.upsert(&[row("m1", "acc", 1.0)]).unwrap();
        store.upsert(&[row("m1", "acc", 0.5)]).unwrap();
        assert_eq!(
            store.stats().unwrap().total_vectors,
            1,
            "mail re-upsert replaces"
        );
    }

    #[test]
    fn wrong_dim_is_validation_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path()).unwrap();
        let mut bad = row("m1", "acc", 1.0);
        bad.vector.truncate(3);
        assert!(matches!(
            store.upsert(&[bad]).unwrap_err(),
            AppError::Validation(_)
        ));
    }

    fn row_dated(id: &str, acc: &str, date_sent: i64) -> VectorRow {
        let mut r = row(id, acc, 1.0);
        r.date_sent = date_sent;
        r
    }

    #[test]
    fn ann_respects_account_and_date_filters() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path()).unwrap();
        store
            .upsert(&[
                row_dated("m1", "acc-a", 1_000),
                row_dated("m2", "acc-b", 2_000),
                row_dated("m3", "acc-a", 3_000),
            ])
            .unwrap();
        let q = vec![1.0f32; VECTOR_DIM];

        // Account filter keeps only acc-a's two chunks.
        let hits = store
            .ann(
                &q,
                10,
                AnnFilter {
                    account_id: Some("acc-a".into()),
                    ..AnnFilter::default()
                },
            )
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|h| h.mail_id == "m1" || h.mail_id == "m3"));

        // Inclusive date window [2_000, 3_000] keeps m2 and m3.
        let hits = store
            .ann(
                &q,
                10,
                AnnFilter {
                    date_from: Some(2_000),
                    date_to: Some(3_000),
                    ..AnnFilter::default()
                },
            )
            .unwrap();
        let ids: std::collections::HashSet<&str> =
            hits.iter().map(|h| h.mail_id.as_str()).collect();
        assert_eq!(ids, ["m2", "m3"].into_iter().collect());
    }

    fn attachment_row(attachment_id: &str, mail_id: &str, idx: i32) -> VectorRow {
        VectorRow {
            chunk_id: format!("{attachment_id}:{idx}"),
            mail_id: mail_id.into(),
            chunk_index: idx,
            account_id: "acc".into(),
            from_email: "a@x.com".into(),
            date_sent: 1000,
            subject: "s".into(),
            snippet: "sn".into(),
            embedding_model: "bge-m3".into(),
            vector: vec![1.0; VECTOR_DIM],
        }
    }

    #[test]
    fn attachment_chunks_are_independent_of_body_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path()).unwrap();
        // A mail body chunk + two attachment chunks sharing the mail_id.
        store.upsert(&[row("m1", "acc", 1.0)]).unwrap();
        store
            .upsert_attachment_chunks(
                "att1",
                &[
                    attachment_row("att1", "m1", 0),
                    attachment_row("att1", "m1", 1),
                ],
            )
            .unwrap();
        assert_eq!(store.stats().unwrap().total_vectors, 3);

        // Re-embedding the mail body must not wipe the attachment's chunks.
        store.upsert(&[row("m1", "acc", 0.5)]).unwrap();
        assert_eq!(
            store.stats().unwrap().total_vectors,
            3,
            "body re-upsert keeps attachment chunks"
        );

        // Deleting the attachment removes only its chunks; the body survives.
        store.delete_attachment_chunks("att1").unwrap();
        assert_eq!(store.stats().unwrap().total_vectors, 1);
        assert!(store.contains_mail("m1"));
    }

    #[test]
    fn rebuild_replaces_all_rows_and_stamps_time() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path()).unwrap();
        // A freshly opened store has never been rebuilt.
        assert!(store.stats().unwrap().last_rebuild_at.is_none());
        store.upsert(&[row("m1", "acc", 1.0)]).unwrap();

        let stats = store
            .rebuild([row("m2", "acc", 1.0), row("m3", "acc", 1.0)].into_iter())
            .unwrap();
        assert_eq!(stats.total_vectors, 2);
        assert!(stats.last_rebuild_at.is_some());
        // The rebuild stream is authoritative: the old row is gone.
        assert!(!store.contains_mail("m1"));
        assert!(store.contains_mail("m2"));
        assert!(store.contains_mail("m3"));
    }
}
