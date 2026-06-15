//! `StorageFacade` — the three-layer storage front (T019, A3).
//!
//! * SQLite ([`Db`]) — authoritative metadata.
//! * [`VectorStore`] — derived semantic index (rebuildable).
//! * [`DiskBlobStore`] — attachment bytes on disk.
//!
//! `AppState` holds exactly one of these and reaches each layer through the
//! accessors, so call sites read `state.storage.db()` / `.vectors()` / `.blobs()`
//! instead of three separate fields.

use std::sync::Arc;

use super::{Db, DiskBlobStore};
use crate::config::Paths;
use crate::error::AppResult;
use crate::vector::VectorStore;

/// Unified handle over the three storage layers. Cheap to clone.
#[derive(Clone)]
pub struct StorageFacade {
    db: Db,
    vectors: Arc<VectorStore>,
    blobs: DiskBlobStore,
}

impl StorageFacade {
    /// Initialise every layer in order: directories → SQLite (+ migrations) →
    /// vector index → blob store.
    pub async fn open(paths: &Paths) -> AppResult<Self> {
        paths.ensure_dirs()?;
        let db = Db::connect(&paths.db).await?;
        db.run_migrations().await?;
        let vectors = Arc::new(VectorStore::open(&paths.vectors)?);
        let blobs = DiskBlobStore::new(paths.root.clone());
        Ok(Self { db, vectors, blobs })
    }

    /// In-memory variant for tests (temp dirs for vectors/blobs).
    #[cfg(test)]
    pub async fn open_in_memory() -> AppResult<Self> {
        use crate::util::new_uuid;
        let db = Db::connect_in_memory().await?;
        db.run_migrations().await?;
        let tmp = std::env::temp_dir().join(format!("seekermail-test-{}", new_uuid()));
        let vectors = Arc::new(VectorStore::open(&tmp.join("vectors"))?);
        let blobs = DiskBlobStore::new(tmp);
        Ok(Self { db, vectors, blobs })
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn vectors(&self) -> &VectorStore {
        &self.vectors
    }

    pub fn blobs(&self) -> &DiskBlobStore {
        &self.blobs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_in_memory_wires_all_three_layers() {
        let s = StorageFacade::open_in_memory().await.unwrap();
        // DB usable.
        assert_eq!(s.db().health_check().await.unwrap(), 0);
        // Vector index usable.
        assert_eq!(s.vectors().stats().unwrap().total_vectors, 0);
        // Blob store usable.
        let r = s
            .blobs()
            .write_attachment("acc", "mail", 2026, 6, "f.pdf", b"x")
            .await
            .unwrap();
        assert!(r.relative_path.contains("attachments"));
    }
}
