//! `email_vectors` schema constants (T019, 01 §LanceDB Schema).
//!
//! The vector index is **derived** state (01 Overview): SQLite is authoritative
//! and the index can be rebuilt at any time from `mails` rows whose
//! `embedding_status = 'indexed'`. One row PER CHUNK, keyed by
//! `chunk_id = "{mail_id}:{chunk_index}"`.

/// Embedding dimensionality (bge-m3 / GTE). Every [`super::VectorRow::vector`]
/// must be exactly this long.
pub const VECTOR_DIM: usize = 1024;

/// IVF-PQ tuning the LanceDB backend would use (kept here so the values live with
/// the schema even though the v0.2 brute-force backend doesn't need them).
pub const IVF_NUM_PARTITIONS: usize = 256;
pub const IVF_NUM_SUB_VECTORS: usize = 48;

/// Fraction of new rows (relative to total) that triggers an index rebuild.
pub const REBUILD_THRESHOLD: f32 = 0.15;

/// Minimum row count before an ANN index is worth building (avoids IVF on an
/// almost-empty table).
pub const INDEX_MIN_ROWS: usize = 1000;

/// Column names, mirroring the Arrow schema the LanceDB table would declare.
pub mod col {
    pub const CHUNK_ID: &str = "chunk_id";
    pub const MAIL_ID: &str = "mail_id";
    pub const CHUNK_INDEX: &str = "chunk_index";
    pub const ACCOUNT_ID: &str = "account_id";
    pub const FROM_EMAIL: &str = "from_email";
    pub const DATE_SENT: &str = "date_sent";
    pub const SUBJECT: &str = "subject";
    pub const SNIPPET: &str = "snippet";
    pub const EMBEDDING_MODEL: &str = "embedding_model";
    pub const VECTOR: &str = "vector";
}
