//! Pure-Rust HNSW ANN backend for the vector store (analysis/55 §5), compiled
//! only under `--features hnsw`.
//!
//! The default build keeps the self-contained brute-force cosine scan in [`super`].
//! Under this feature, [`super::VectorStore::ann`] lazily builds and caches an
//! [`HnswIndex`] from the authoritative `rows` (the JSON snapshot stays the source
//! of truth, and the index is rebuilt on any mutation), turning the O(N) scan into
//! an approximate sub-linear search. Distance is cosine — identical to the
//! brute-force ranker — so the two agree on the vectors HNSW visits, and the score
//! returned (`1 - cosine_distance`) is exactly the cosine similarity the callers
//! already expect.
//!
//! Metadata filtering (account / date) is applied *after* the vector search
//! (post-filtering): the search beam (`ef_search`) over-fetches neighbours, then we
//! keep those matching the [`super::AnnFilter`] and truncate to `k`. No external
//! native stack (Arrow / LanceDB) — `instant-distance` is pure Rust.

use instant_distance::{Builder, HnswMap, Point, Search};

use super::{cosine, AnnFilter, Hit, VectorRow};

/// HNSW build quality (`efConstruction`). Higher = better recall, slower build.
const EF_CONSTRUCTION: usize = 100;
/// HNSW search beam width (`ef`). Bounds (and over-fetches) the candidate pool each
/// query explores before metadata post-filtering and truncation to `k`.
const EF_SEARCH: usize = 256;
/// Fixed RNG seed so a given row set always builds the same graph — reproducible
/// across runs, unlike the crate's default entropy seed.
const BUILD_SEED: u64 = 0x5EED_A11A;

/// One embedding vector wrapped for the HNSW index. Cosine distance
/// (`1 - cosine_similarity`) matches the brute-force ranker exactly.
#[derive(Clone)]
struct EmbeddingPoint(Vec<f32>);

impl Point for EmbeddingPoint {
    fn distance(&self, other: &Self) -> f32 {
        1.0 - cosine(&self.0, &other.0)
    }
}

/// Per-chunk metadata kept alongside the index so results can be filtered and
/// turned back into [`Hit`]s without storing the full vectors twice.
struct ChunkMeta {
    chunk_id: String,
    mail_id: String,
    account_id: String,
    date_sent: i64,
}

/// A built HNSW index over one snapshot of the store's rows. `map` is `None` for an
/// empty corpus (the crate is never asked to build an empty graph).
pub(super) struct HnswIndex {
    map: Option<HnswMap<EmbeddingPoint, u32>>,
    meta: Vec<ChunkMeta>,
}

impl HnswIndex {
    /// Build the index from the current rows. O(N log N); the caller caches the
    /// result and rebuilds only when the rows change.
    pub(super) fn build(rows: &[VectorRow]) -> Self {
        if rows.is_empty() {
            return Self {
                map: None,
                meta: Vec::new(),
            };
        }
        let mut points = Vec::with_capacity(rows.len());
        let mut values = Vec::with_capacity(rows.len());
        let mut meta = Vec::with_capacity(rows.len());
        for (i, r) in rows.iter().enumerate() {
            points.push(EmbeddingPoint(r.vector.clone()));
            values.push(i as u32);
            meta.push(ChunkMeta {
                chunk_id: r.chunk_id.clone(),
                mail_id: r.mail_id.clone(),
                account_id: r.account_id.clone(),
                date_sent: r.date_sent,
            });
        }
        let map = Builder::default()
            .ef_construction(EF_CONSTRUCTION)
            .ef_search(EF_SEARCH)
            .seed(BUILD_SEED)
            .build(points, values);
        Self {
            map: Some(map),
            meta,
        }
    }

    /// k-nearest neighbours honouring the metadata filter (post-filtered). Results
    /// are score-descending (nearest first), each `Hit.score` the cosine similarity.
    pub(super) fn search(&self, query: &[f32], k: usize, filter: &AnnFilter) -> Vec<Hit> {
        let Some(map) = &self.map else {
            return Vec::new();
        };
        if k == 0 {
            return Vec::new();
        }
        let probe = EmbeddingPoint(query.to_vec());
        let mut search = Search::default();
        let mut hits: Vec<Hit> = Vec::with_capacity(k);
        // The beam yields candidates in increasing distance (decreasing similarity);
        // keep the filter-matching ones until we have `k`.
        for item in map.search(&probe, &mut search) {
            let meta = &self.meta[*item.value as usize];
            if !filter.matches_parts(&meta.account_id, meta.date_sent) {
                continue;
            }
            hits.push(Hit {
                chunk_id: meta.chunk_id.clone(),
                mail_id: meta.mail_id.clone(),
                score: 1.0 - item.distance,
            });
            if hits.len() >= k {
                break;
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit vector pointing mostly along axis `i` (so nearest-neighbour outcomes
    /// are obvious), tagged with account + date for filter tests.
    fn row(chunk: &str, mail: &str, account: &str, date: i64, axis: usize) -> VectorRow {
        let mut v = vec![0.0f32; super::super::schema::VECTOR_DIM];
        v[axis] = 1.0;
        VectorRow {
            chunk_id: chunk.into(),
            mail_id: mail.into(),
            chunk_index: 0,
            account_id: account.into(),
            from_email: "peer@x.com".into(),
            date_sent: date,
            subject: "s".into(),
            snippet: "s".into(),
            embedding_model: "bge-m3".into(),
            vector: v,
        }
    }

    fn query(axis: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; super::super::schema::VECTOR_DIM];
        v[axis] = 1.0;
        v
    }

    #[test]
    fn empty_index_returns_no_hits() {
        let idx = HnswIndex::build(&[]);
        assert!(idx.search(&query(0), 5, &AnnFilter::default()).is_empty());
    }

    #[test]
    fn finds_the_nearest_vector() {
        let rows = vec![
            row("a:0", "a", "acc", 10, 0),
            row("b:0", "b", "acc", 10, 1),
            row("c:0", "c", "acc", 10, 2),
        ];
        let idx = HnswIndex::build(&rows);
        let hits = idx.search(&query(1), 1, &AnnFilter::default());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].mail_id, "b",
            "axis-1 query must return the axis-1 mail"
        );
        assert!(hits[0].score > 0.99, "exact match scores ~1.0");
    }

    #[test]
    fn honours_k_and_score_order() {
        let rows = vec![
            row("a:0", "a", "acc", 10, 0),
            row("b:0", "b", "acc", 10, 1),
            row("c:0", "c", "acc", 10, 2),
            row("d:0", "d", "acc", 10, 3),
        ];
        let idx = HnswIndex::build(&rows);
        let hits = idx.search(&query(0), 2, &AnnFilter::default());
        assert_eq!(hits.len(), 2, "k caps the result count");
        for pair in hits.windows(2) {
            assert!(pair[0].score >= pair[1].score, "nearest first");
        }
    }

    #[test]
    fn respects_account_filter() {
        let rows = vec![
            row("a:0", "a", "acc1", 10, 0),
            row("b:0", "b", "acc2", 10, 0),
        ];
        let idx = HnswIndex::build(&rows);
        let filter = AnnFilter {
            account_id: Some("acc2".into()),
            date_from: None,
            date_to: None,
        };
        let hits = idx.search(&query(0), 5, &filter);
        assert!(!hits.is_empty());
        assert!(
            hits.iter().all(|h| h.mail_id == "b"),
            "account filter excludes acc1"
        );
    }

    #[test]
    fn respects_date_filter() {
        let rows = vec![
            row("old:0", "old", "acc", 100, 0),
            row("new:0", "new", "acc", 5_000, 0),
        ];
        let idx = HnswIndex::build(&rows);
        let filter = AnnFilter {
            account_id: None,
            date_from: Some(1_000),
            date_to: None,
        };
        let hits = idx.search(&query(0), 5, &filter);
        assert!(
            hits.iter().all(|h| h.mail_id == "new"),
            "date_from excludes the old mail"
        );
    }
}
