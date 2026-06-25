//! Role context assembly — the GTE context window (T074, dev/06 §5).
//!
//! [`assemble_role_context`] is a pure Rust **internal library function** (not a
//! `tauri::command`). Role analysis (module D, T070/T072) and reply generation
//! (module E, T077+) call it to build the grounded prompt inputs before talking
//! to any provider:
//!
//! 1. **Role preamble** from `accounts.role_type` / `accounts.role_description`.
//! 2. **Safety preamble** (always present): never fabricate commitments, flag
//!    uncertainty, defer out-of-scope asks to the human.
//! 3. **GTE context** — top-K relevant prior mails via the two-stage ANN query
//!    (SQLite pre-filter → vector ANN → per-mail max-score aggregation).
//! 4. **Target mail** (B1-sanitized `body_text`, never raw HTML).
//! 5. **Recent thread snippets** (latest 5 of the same thread).
//! 6. **Contact history stats** from `contacts` (D2 sales analysis input).
//!
//! A **context packer** trims everything to the caller's token budget with the
//! fixed priority order from dev/06 §5 — safety > target mail > recent thread >
//! GTE context (> style exemplars, which are E5's card and not assembled here).
//! Only when even the minimum (preambles + target mail) doesn't fit does it
//! return [`AppError::AiContextTooLong`]; callers then raise a T5 query instead
//! of truncating the target mail.
//!
//! The `mail_id`s of every kept chunk are recorded in
//! [`RoleContext::knowledge_refs`] for the `ai_drafts` / `ai_decisions` audit
//! columns (dev/06 §9).
//!
//! **Logging red-line (dev/09 §5):** this module logs identifiers and counts
//! only — never `body_text`, snippets, or any other mail content.

use std::collections::{HashMap, HashSet};

use sqlx::Row;

use crate::error::{AppError, AppResult};
use crate::search::fts5;
use crate::state::AppState;
use crate::util::truncate_chars;
use crate::vector::AnnFilter;

use super::types::Capability;

/// The fixed safety preamble (dev/06 §5 item 2). Always included in the system
/// prompt and always counted first by the context packer.
pub const SAFETY_PREAMBLE: &str = "Operating rules: never fabricate commitments, prices, dates, names, or facts. \
When information is missing or uncertain, say so explicitly instead of guessing. \
If a request falls outside your assigned role or authority, defer to the human operator instead of acting on it.";

/// Default ANN fan-out after per-mail aggregation (card §3).
pub const DEFAULT_TOP_K: usize = 10;
/// Default cosine floor for GTE chunks (dev/01 §LanceDB).
pub const DEFAULT_MIN_SCORE: f32 = 0.35;

/// Chunk-level recall multiplier before per-mail aggregation (dev/01 §LanceDB:
/// `top_k * 3` chunks recalled, merged per mail, best chunk kept).
const ANN_RECALL_FACTOR: usize = 3;
/// Same-thread mails included, newest first, trigger excluded (F_D1 §4.2).
const THREAD_MAILS_LIMIT: i64 = 5;
/// Knowledge-chunk snippet length in characters (card §3). `pub(crate)` so the
/// MCE question path hydrates semantic items to the same snippet length.
pub(crate) const CHUNK_SNIPPET_CHARS: usize = 200;
/// Per-mail body cap for thread snippets, so one verbose reply can't starve the
/// rest of the thread out of the budget. The target mail is never capped here.
const THREAD_BODY_CHARS: usize = 800;
/// Characters of "subject + body" fed to the query embedder; bge-m3 sees ~8K
/// tokens of input, so this comfortably covers the semantically dense head.
const EMBED_QUERY_CHARS: usize = 4000;

/// Inputs for [`assemble_role_context`].
///
/// `mail_id` / `account_id` / `thread_id` are the UUID strings used everywhere
/// in the schema (`TEXT` primary keys, dev/01) — the same form callers already
/// hold from IPC params and repo rows.
#[derive(Debug, Clone)]
pub struct RoleContextParams {
    /// The trigger mail being analyzed / replied to.
    pub mail_id: String,
    /// The account (digital employee) doing the work.
    pub account_id: String,
    /// Same-thread snippets are assembled only when this is `Some` — callers
    /// pass the trigger mail's `thread_id` when they want thread context.
    pub thread_id: Option<String>,
    /// Total token budget for the assembled context (typically derived from
    /// the resolved provider's `context_window()` minus generation headroom).
    pub token_budget: usize,
    /// Mails kept after per-mail ANN aggregation.
    pub top_k: usize,
    /// Cosine floor below which GTE hits are discarded.
    pub min_score: f32,
    /// What the context is for; gates the contact-history lookup and lets the
    /// audit row record intent.
    pub capability: Capability,
}

impl RoleContextParams {
    /// Params with the card defaults (`top_k = 10`, `min_score = 0.35`, no
    /// thread context).
    pub fn new(
        mail_id: impl Into<String>,
        account_id: impl Into<String>,
        token_budget: usize,
        capability: Capability,
    ) -> Self {
        Self {
            mail_id: mail_id.into(),
            account_id: account_id.into(),
            thread_id: None,
            token_budget,
            top_k: DEFAULT_TOP_K,
            min_score: DEFAULT_MIN_SCORE,
            capability,
        }
    }
}

/// One GTE knowledge chunk kept by the packer (card §3).
#[derive(Debug, Clone)]
pub struct KnowledgeChunk {
    pub mail_id: String,
    pub chunk_index: i32,
    pub score: f32,
    /// 200-char preview of the source mail (the `mails.snippet` column).
    pub snippet: String,
    pub date_sent: i64,
}

/// One mail rendered for prompt inclusion (target mail or thread member).
#[derive(Debug, Clone)]
pub struct MailSnippet {
    pub mail_id: String,
    pub subject: String,
    pub from_email: String,
    pub date_sent: i64,
    /// Sanitized plain text (`mails.body_text`, B1). Full-length for the target
    /// mail; capped at [`THREAD_BODY_CHARS`] for thread members.
    pub body: String,
}

/// Counterparty history from the `contacts` table (D2 sales input, F_D2 §4.2).
#[derive(Debug, Clone)]
pub struct ContactHistory {
    /// Normalised lowercase counterparty address the stats belong to.
    pub email: String,
    pub data: ContactHistoryData,
}

/// The stats columns read from `contacts` (dev/01 §contacts).
#[derive(Debug, Clone)]
pub struct ContactHistoryData {
    pub interaction_count: i64,
    pub reply_count: i64,
    pub avg_reply_hours: Option<f64>,
    /// JSON AI-extracted style notes, passed through opaquely.
    pub style_notes: Option<String>,
}

/// The assembled, budget-fitted prompt inputs.
#[derive(Debug, Clone)]
pub struct RoleContext {
    /// Persona line built from `accounts.role_type` / `role_description`.
    pub role_preamble: String,
    /// Always [`SAFETY_PREAMBLE`]; carried here so callers build the system
    /// prompt from this struct alone.
    pub safety_preamble: String,
    /// The trigger mail, sanitized and **never truncated** at this layer.
    pub target_mail: MailSnippet,
    /// Latest same-thread mails (≤ 5, newest first, trigger excluded) that fit
    /// the budget.
    pub thread_mails: Vec<MailSnippet>,
    /// GTE chunks that fit the budget, score-descending.
    pub chunks: Vec<KnowledgeChunk>,
    /// Deduplicated `mail_id`s of every kept chunk, score-descending — written
    /// to `ai_drafts.knowledge_refs` / `ai_decisions.knowledge_refs`.
    pub knowledge_refs: Vec<String>,
    /// Counterparty stats; `None` on first contact (callers handle gracefully).
    pub contact_history: Option<ContactHistory>,
    /// Estimated tokens of everything kept; always `<= token_budget`.
    pub total_tokens_used: usize,
}

impl RoleContext {
    /// Role + safety preamble joined for `ChatRequest::system` (dev/06 §5).
    pub fn system_preamble(&self) -> String {
        format!("{}\n\n{}", self.role_preamble, self.safety_preamble)
    }
}

/// Assemble the grounded context window for one `(account, mail, capability)`
/// call. See the module docs for the full pipeline and the packing rules.
pub async fn assemble_role_context(
    state: &AppState,
    params: &RoleContextParams,
) -> AppResult<RoleContext> {
    let db = state.storage.db().pool();

    // 1) Role preamble from the account row.
    let account =
        sqlx::query("SELECT email, role_type, role_description FROM accounts WHERE id = ?")
            .bind(&params.account_id)
            .fetch_optional(db)
            .await
            .map_err(crate::storage::map_sqlx_err)?
            .ok_or(AppError::NotFound)?;
    let role_preamble = build_role_preamble(
        &account.get::<String, _>("email"),
        &account.get::<String, _>("role_type"),
        account
            .get::<Option<String>, _>("role_description")
            .as_deref(),
    );

    // 2) Target mail (must belong to the account; body already B1-sanitized).
    let target_row = sqlx::query(
        "SELECT subject, from_email, date_sent, COALESCE(body_text, '') AS body \
         FROM mails WHERE id = ? AND account_id = ? AND is_deleted = 0",
    )
    .bind(&params.mail_id)
    .bind(&params.account_id)
    .fetch_optional(db)
    .await
    .map_err(crate::storage::map_sqlx_err)?
    .ok_or(AppError::NotFound)?;
    let target_mail = MailSnippet {
        mail_id: params.mail_id.clone(),
        subject: target_row.get("subject"),
        from_email: target_row.get("from_email"),
        date_sent: target_row.get("date_sent"),
        body: target_row.get("body"),
    };

    // 3) Hard minimum gate (dev/06 §5): preambles + the untruncated target mail
    // must fit, or the whole call is AI_CONTEXT_TOO_LONG (caller raises a T5
    // query — this layer never truncates the target mail).
    let mut used = estimate_tokens(SAFETY_PREAMBLE)
        + estimate_tokens(&role_preamble)
        + snippet_tokens(&target_mail);
    if used > params.token_budget {
        return Err(AppError::AiContextTooLong);
    }

    // 4) GTE retrieval — two-stage query (dev/01 §LanceDB).
    let all_chunks = retrieve_chunks(state, params, &target_mail).await?;

    // 5) Same-thread snippets (latest 5, newest first, trigger excluded).
    let all_thread_mails = match &params.thread_id {
        Some(thread_id) => fetch_thread_mails(state, thread_id, &params.mail_id).await?,
        None => Vec::new(),
    };

    // 6) Counterparty history for analysis capabilities (F_D2 §4.2).
    let contact_history = match params.capability {
        Capability::RiskReason | Capability::DraftReply => {
            fetch_contact_history(state, &target_mail.from_email).await?
        }
        Capability::Summarize | Capability::StyleProfile => None,
    };

    // 7) Context packer — fixed priority order (dev/06 §5): the preambles and
    // target mail are already counted; thread mails pack next (newest first),
    // then GTE chunks by score descending. Packing stops at the first item
    // that no longer fits, so a long thread degrades gracefully.
    let mut thread_mails = Vec::new();
    for mail in all_thread_mails {
        let cost = snippet_tokens(&mail);
        if used + cost > params.token_budget {
            break;
        }
        used += cost;
        thread_mails.push(mail);
    }
    let mut chunks = Vec::new();
    for chunk in all_chunks {
        let cost = estimate_tokens(&chunk.snippet);
        if used + cost > params.token_budget {
            break;
        }
        used += cost;
        chunks.push(chunk);
    }

    // 8) knowledge_refs: kept chunks' mail_ids, deduplicated, score-descending
    // (chunks are already sorted by score).
    let mut knowledge_refs: Vec<String> = Vec::with_capacity(chunks.len());
    for c in &chunks {
        if !knowledge_refs.iter().any(|id| id == &c.mail_id) {
            knowledge_refs.push(c.mail_id.clone());
        }
    }

    // Identifiers and counts only — never mail content (dev/09 §5).
    tracing::info!(
        event = "role_context_assembled",
        mail_id = %params.mail_id,
        account_id = %params.account_id,
        capability = params.capability.as_str(),
        chunks = chunks.len(),
        thread_mails = thread_mails.len(),
        tokens = used,
        budget = params.token_budget,
        "assembled role context"
    );

    Ok(RoleContext {
        role_preamble,
        safety_preamble: SAFETY_PREAMBLE.to_string(),
        target_mail,
        thread_mails,
        chunks,
        knowledge_refs,
        contact_history,
        total_tokens_used: used,
    })
}

/// One mail ranked by the semantic leg, before any rendering: the mail id, the
/// index of its best-scoring chunk, and that chunk's cosine score. This is the
/// raw output of leg B; each caller hydrates whatever shape it needs from it.
#[derive(Debug, Clone)]
pub(crate) struct ScoredMail {
    pub mail_id: String,
    pub chunk_index: i32,
    pub score: f32,
}

/// Hybrid retrieval leg B (analysis/54 §3.2 + analysis/55 §5), decoupled from any
/// caller's shape. Runs two retrievers over the same active scope and fuses them:
///   * **dense** — vector ANN (per-mail max cosine, gated by the `min_score`
///     floor);
///   * **sparse** — FTS5 / BM25 keyword probe over the same mailbox.
///
/// Reciprocal rank fusion merges the two into one score-descending top-K of
/// [`ScoredMail`]s (fused score normalised to (0, 1]); hydration is the caller's
/// job. Either signal alone can surface a mail, so exact terms / names / ids the
/// embedding blurs and topics the keywords miss both get found — one path, two
/// signals.
///
/// Both the anchored reply path ([`assemble_role_context`], which excludes the
/// trigger mail so it can never ground itself) and the anchorless chat path
/// (the MCE question path, which has nothing to exclude) call this, so there is
/// exactly **one** semantic retrieval implementation in the product — the whole
/// point of folding the two fetch paths into one engine.
pub(crate) async fn retrieve_scored(
    state: &AppState,
    account_id: &str,
    query_text: &str,
    exclude_mail_id: Option<&str>,
    top_k: usize,
    min_score: f32,
) -> AppResult<Vec<ScoredMail>> {
    // Each leg recalls a wider pool than the final top_k, so a mail ranked by
    // only one signal still gets a fair chance in the fusion.
    let pool = top_k.saturating_mul(ANN_RECALL_FACTOR).max(top_k).max(1);

    // Leg B1 — dense vector ranking (cosine, gated by the score floor).
    let dense = retrieve_dense(
        state,
        account_id,
        query_text,
        exclude_mail_id,
        pool,
        min_score,
    )
    .await?;

    // Leg B2 — sparse BM25 ranking over the same active scope: a keyword probe so
    // exact terms, names, ids, and rare tokens the embedding blurs still surface.
    let sparse = fts5::bm25_ranked_ids(
        state.storage.db().pool(),
        account_id,
        exclude_mail_id,
        query_text,
        pool,
    )
    .await?;

    // Fuse the two rankings into one ordered top-K (reciprocal rank fusion).
    Ok(fuse_rrf(dense, sparse, top_k))
}

/// Leg B1 — dense (vector) retrieval: SQLite pre-filter (account, optional
/// `exclude`, not archived/deleted) → vector ANN at `pool * 3` chunk recall →
/// per-mail max cosine → `min_score` floor → ranked by cosine, capped at `pool`.
/// Each [`ScoredMail`] carries its per-mail best-chunk index and cosine; the
/// fuser later replaces the score with the fused value.
async fn retrieve_dense(
    state: &AppState,
    account_id: &str,
    query_text: &str,
    exclude_mail_id: Option<&str>,
    pool: usize,
    min_score: f32,
) -> AppResult<Vec<ScoredMail>> {
    let db = state.storage.db().pool();

    // Stage 1 — SQLite pre-filter (card §3 step 2; no date restriction).
    let candidate_rows = match exclude_mail_id {
        Some(exclude) => sqlx::query(
            "SELECT id FROM mails WHERE account_id = ? AND id != ? \
             AND is_archived = 0 AND is_deleted = 0",
        )
        .bind(account_id)
        .bind(exclude),
        None => sqlx::query(
            "SELECT id FROM mails WHERE account_id = ? \
             AND is_archived = 0 AND is_deleted = 0",
        )
        .bind(account_id),
    }
    .fetch_all(db)
    .await
    .map_err(crate::storage::map_sqlx_err)?;
    if candidate_rows.is_empty() {
        return Ok(Vec::new());
    }
    let candidates: HashSet<String> = candidate_rows
        .iter()
        .map(|r| r.get::<String, _>("id"))
        .collect();

    // Embed the query head (local ONNX, never a provider).
    let query_vec = state
        .embedder
        .embed_blocking(truncate_chars(query_text, EMBED_QUERY_CHARS))
        .await
        .map_err(|e| AppError::AiUnreachable(format!("context embed failed: {e}")))?;

    // Stage 2 — vector ANN scoped to the account. Index-level failures surface
    // as GTE_INDEX_CORRUPT (card §6); the underlying cause is logged without
    // any content.
    let hits = state
        .storage
        .vectors()
        .ann(
            &query_vec,
            pool.saturating_mul(ANN_RECALL_FACTOR).max(1),
            AnnFilter {
                account_id: Some(account_id.to_string()),
                date_from: None,
                date_to: None,
            },
        )
        .map_err(|e| {
            tracing::warn!(event = "gte_ann_failed", error = %e, "vector ann failed");
            AppError::GteCorrupt
        })?;

    // Aggregate chunks → best chunk per mail; gate by candidates + min_score.
    let mut best: HashMap<String, (i32, f32)> = HashMap::new();
    for h in &hits {
        if h.score < min_score || !candidates.contains(&h.mail_id) {
            continue;
        }
        let entry = best
            .entry(h.mail_id.clone())
            .or_insert((chunk_index_of(&h.chunk_id), f32::MIN));
        if h.score > entry.1 {
            *entry = (chunk_index_of(&h.chunk_id), h.score);
        }
    }

    let mut ranked: Vec<ScoredMail> = best
        .into_iter()
        .map(|(mail_id, (chunk_index, score))| ScoredMail {
            mail_id,
            chunk_index,
            score,
        })
        .collect();
    // Deterministic order: cosine desc, then mail_id asc on ties (M10).
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.mail_id.cmp(&b.mail_id))
    });
    ranked.truncate(pool);
    Ok(ranked)
}

/// Damping constant for reciprocal rank fusion (the standard k = 60).
const RRF_K: f32 = 60.0;

/// Fuse the dense and sparse rankings into one ordered list (reciprocal rank
/// fusion, analysis/55 §5). A mail's fused weight sums `1 / (RRF_K + rank)` over
/// each list it appears in (rank 0-based), so agreement between the two signals
/// lifts a mail while either signal alone still surfaces it. The weight is
/// normalised to (0, 1] (best = 1.0) so the score stays a comparable relevance
/// proxy and the downstream score-descending invariant holds. The dense best-
/// chunk index is preserved; sparse-only hits carry chunk 0. Deterministic: ties
/// break on `mail_id`.
fn fuse_rrf(dense: Vec<ScoredMail>, sparse: Vec<String>, top_k: usize) -> Vec<ScoredMail> {
    if dense.is_empty() && sparse.is_empty() {
        return Vec::new();
    }

    let mut weight: HashMap<String, f32> = HashMap::new();
    let mut chunk_of: HashMap<String, i32> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (rank, d) in dense.iter().enumerate() {
        chunk_of.insert(d.mail_id.clone(), d.chunk_index);
        *weight.entry(d.mail_id.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f32);
        order.push(d.mail_id.clone());
    }
    for (rank, id) in sparse.iter().enumerate() {
        if !weight.contains_key(id) {
            order.push(id.clone());
        }
        *weight.entry(id.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f32);
    }

    let max = weight
        .values()
        .copied()
        .fold(f32::MIN, f32::max)
        .max(f32::MIN_POSITIVE);
    let mut ranked: Vec<ScoredMail> = order
        .into_iter()
        .map(|mail_id| {
            let score = (weight[&mail_id] / max).clamp(0.0, 1.0);
            let chunk_index = chunk_of.get(&mail_id).copied().unwrap_or(0);
            ScoredMail {
                mail_id,
                chunk_index,
                score,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.mail_id.cmp(&b.mail_id))
    });
    ranked.truncate(top_k);
    ranked
}

/// Anchored leg B: build the semantic query from the trigger mail, then hydrate
/// the ranked hits into [`KnowledgeChunk`]s (200-char snippet + date) for the
/// reply/analysis prompt. A behaviour-preserving wrapper over [`retrieve_scored`].
async fn retrieve_chunks(
    state: &AppState,
    params: &RoleContextParams,
    target: &MailSnippet,
) -> AppResult<Vec<KnowledgeChunk>> {
    // Embed "subject + body head" as the query (truncation happens inside).
    let query_text = format!("{}\n{}", target.subject, target.body);
    let ranked = retrieve_scored(
        state,
        &params.account_id,
        &query_text,
        Some(&params.mail_id),
        params.top_k,
        params.min_score,
    )
    .await?;
    if ranked.is_empty() {
        return Ok(Vec::new());
    }

    // Hydrate snippet + date_sent from SQLite (the snippet column is already
    // the 200-char preview written at ingest).
    let db = state.storage.db().pool();
    let placeholders = vec!["?"; ranked.len()].join(",");
    let sql = format!(
        "SELECT id, COALESCE(snippet, '') AS snippet, date_sent \
         FROM mails WHERE id IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql);
    for sm in &ranked {
        q = q.bind(&sm.mail_id);
    }
    let rows = q
        .fetch_all(db)
        .await
        .map_err(crate::storage::map_sqlx_err)?;
    let mut meta: HashMap<String, (String, i64)> = rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("id"),
                (r.get::<String, _>("snippet"), r.get::<i64, _>("date_sent")),
            )
        })
        .collect();

    Ok(ranked
        .into_iter()
        .filter_map(|sm| {
            meta.remove(&sm.mail_id)
                .map(|(snippet, date_sent)| KnowledgeChunk {
                    mail_id: sm.mail_id,
                    chunk_index: sm.chunk_index,
                    score: sm.score,
                    snippet: truncate_chars(&snippet, CHUNK_SNIPPET_CHARS),
                    date_sent,
                })
        })
        .collect())
}

/// Latest [`THREAD_MAILS_LIMIT`] same-thread mails, newest first, excluding the
/// trigger mail (F_D1 §4.2). Bodies are capped at [`THREAD_BODY_CHARS`].
async fn fetch_thread_mails(
    state: &AppState,
    thread_id: &str,
    trigger_mail_id: &str,
) -> AppResult<Vec<MailSnippet>> {
    let rows = sqlx::query(
        "SELECT id, subject, from_email, date_sent, COALESCE(body_text, '') AS body \
         FROM mails WHERE thread_id = ? AND id != ? AND is_deleted = 0 \
         ORDER BY date_sent DESC LIMIT ?",
    )
    .bind(thread_id)
    .bind(trigger_mail_id)
    .bind(THREAD_MAILS_LIMIT)
    .fetch_all(state.storage.db().pool())
    .await
    .map_err(crate::storage::map_sqlx_err)?;
    Ok(rows
        .iter()
        .map(|r| MailSnippet {
            mail_id: r.get("id"),
            subject: r.get("subject"),
            from_email: r.get("from_email"),
            date_sent: r.get("date_sent"),
            body: truncate_chars(&r.get::<String, _>("body"), THREAD_BODY_CHARS),
        })
        .collect())
}

/// Counterparty stats from `contacts`; `None` when there is no row (first
/// contact — D2: the model analyses on the current mail alone).
async fn fetch_contact_history(
    state: &AppState,
    from_email: &str,
) -> AppResult<Option<ContactHistory>> {
    let email = from_email.trim().to_lowercase();
    let row = sqlx::query(
        "SELECT interaction_count, reply_count, avg_reply_hours, style_notes \
         FROM contacts WHERE email = ?",
    )
    .bind(&email)
    .fetch_optional(state.storage.db().pool())
    .await
    .map_err(crate::storage::map_sqlx_err)?;
    Ok(row.map(|r| ContactHistory {
        email,
        data: ContactHistoryData {
            interaction_count: r.get("interaction_count"),
            reply_count: r.get("reply_count"),
            avg_reply_hours: r.get("avg_reply_hours"),
            style_notes: r.get("style_notes"),
        },
    }))
}

/// Persona line from the account's role columns (dev/06 §5 item 1). `pub(crate)`
/// so the MCE can build the same persona for the anchorless question path.
pub(crate) fn build_role_preamble(
    email: &str,
    role_type: &str,
    role_description: Option<&str>,
) -> String {
    let mut preamble =
        format!("You are the dedicated {role_type} assistant for the mailbox {email}.");
    if let Some(description) = role_description {
        let description = description.trim();
        if !description.is_empty() {
            preamble.push_str(" Your assignment: ");
            preamble.push_str(description);
        }
    }
    preamble
}

/// Conservative token estimate: 1 token ≈ 4 bytes (card §6); never zero so an
/// item always carries a cost. `pub(crate)` so the MCE packs to the same budget
/// arithmetic as the anchored path.
pub(crate) fn estimate_tokens(s: &str) -> usize {
    (s.len() / 4).max(1)
}

/// Budget cost of one mail snippet (subject + body).
fn snippet_tokens(m: &MailSnippet) -> usize {
    estimate_tokens(&m.subject) + estimate_tokens(&m.body)
}

/// Chunk index from a `"{mail_id}:{index}"` chunk id; 0 when unparsable.
fn chunk_index_of(chunk_id: &str) -> i32 {
    chunk_id
        .rsplit(':')
        .next()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::now_unix;
    use crate::vector::VectorRow;

    /// Insert an account with explicit role columns.
    async fn seed_account(state: &AppState, id: &str, role_type: &str, role_desc: Option<&str>) {
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, 'X', 'imap', 'slate', 'W', ?, ?, 0, 0)",
        )
        .bind(id)
        .bind(format!("{id}@x.com"))
        .bind(role_type)
        .bind(role_desc)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// Insert a thread row (FKs are ON, so mails.thread_id needs a parent).
    async fn seed_thread(state: &AppState, id: &str, acc: &str) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, latest_date, \
                 created_at, updated_at) \
             VALUES (?, ?, 'Thread', '[]', 0, 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// Insert one mail row (no vector).
    async fn seed_mail(
        state: &AppState,
        id: &str,
        acc: &str,
        thread: Option<&str>,
        from_email: &str,
        body: &str,
        date_sent: i64,
    ) {
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, embedding_status, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, '[]', ?, ?, ?, ?, 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(thread)
        .bind(format!("<{id}@x>"))
        .bind(format!("Subject {id}"))
        .bind(from_email)
        .bind(date_sent)
        .bind(date_sent)
        .bind(body)
        .bind(truncate_chars(body, 200))
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// Embed `text` and upsert it as the mail's chunk(s) in the vector store.
    async fn index_mail(state: &AppState, id: &str, acc: &str, texts: &[&str]) {
        let rows: Vec<VectorRow> = texts
            .iter()
            .enumerate()
            .map(|(i, text)| VectorRow {
                chunk_id: format!("{id}:{i}"),
                mail_id: id.into(),
                chunk_index: i as i32,
                account_id: acc.into(),
                from_email: "peer@x.com".into(),
                date_sent: now_unix(),
                subject: (*text).into(),
                snippet: (*text).into(),
                embedding_model: "bge-m3".into(),
                vector: state.embedder.embed(text).unwrap(),
            })
            .collect();
        state.storage.vectors().upsert(&rows).unwrap();
    }

    /// Insert a contacts row for the counterparty.
    async fn seed_contact(state: &AppState, email: &str, interactions: i64, replies: i64) {
        sqlx::query(
            "INSERT INTO contacts (id, email, first_seen_at, last_seen_at, \
                 interaction_count, reply_count, created_at, updated_at) \
             VALUES (?, ?, 0, 0, ?, ?, 0, 0)",
        )
        .bind(crate::util::new_uuid())
        .bind(email)
        .bind(interactions)
        .bind(replies)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn params(mail: &str, acc: &str, budget: usize) -> RoleContextParams {
        RoleContextParams::new(mail, acc, budget, Capability::RiskReason)
    }

    const TRIGGER_BODY: &str =
        "the quarterly licensing contract renewal terms and the indemnity clause review";

    /// Seed a trigger mail plus two semantically related, indexed prior mails.
    async fn seed_corpus(state: &AppState, acc: &str) {
        seed_account(
            state,
            acc,
            "legal",
            Some("Review inbound contracts for risk."),
        )
        .await;
        seed_mail(
            state,
            "trigger",
            acc,
            None,
            "peer@x.com",
            TRIGGER_BODY,
            now_unix(),
        )
        .await;
        index_mail(state, "trigger", acc, &[TRIGGER_BODY]).await;
        let related1 = "prior contract renewal discussed the licensing terms and indemnity";
        let related2 = "the indemnity clause review from last quarter licensing contract";
        seed_mail(
            state,
            "k1",
            acc,
            None,
            "peer@x.com",
            related1,
            now_unix() - 100,
        )
        .await;
        index_mail(state, "k1", acc, &[related1]).await;
        seed_mail(
            state,
            "k2",
            acc,
            None,
            "peer@x.com",
            related2,
            now_unix() - 200,
        )
        .await;
        index_mail(state, "k2", acc, &[related2]).await;
    }

    #[tokio::test]
    async fn context_assembles_with_chunks_and_refs() {
        let (state, _rx) = AppState::test_state().await;
        seed_corpus(&state, "a").await;

        let p = params("trigger", "a", 100_000);
        let ctx = assemble_role_context(&state, &p).await.unwrap();

        assert!(!ctx.chunks.is_empty(), "related mails should be retrieved");
        assert_eq!(ctx.knowledge_refs.len(), ctx.chunks.len());
        for (chunk, mail_ref) in ctx.chunks.iter().zip(ctx.knowledge_refs.iter()) {
            assert_eq!(&chunk.mail_id, mail_ref, "refs follow chunk score order");
        }
        // Score-descending invariant.
        for pair in ctx.chunks.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
        assert_eq!(ctx.safety_preamble, SAFETY_PREAMBLE);
        assert_eq!(ctx.target_mail.body, TRIGGER_BODY, "target body untouched");
        assert!(ctx.total_tokens_used <= p.token_budget);
    }

    #[test]
    fn fuse_rrf_combines_dense_and_sparse() {
        let dense = vec![
            ScoredMail {
                mail_id: "a".into(),
                chunk_index: 2,
                score: 0.9,
            },
            ScoredMail {
                mail_id: "b".into(),
                chunk_index: 1,
                score: 0.5,
            },
        ];
        let sparse = vec!["b".to_string(), "c".to_string()];

        let fused = fuse_rrf(dense, sparse, 10);
        let ids: Vec<&str> = fused.iter().map(|s| s.mail_id.as_str()).collect();

        // All three surface; the mail both signals agree on ranks first.
        assert_eq!(fused.len(), 3);
        assert_eq!(ids[0], "b", "agreement across dense + sparse wins");
        assert!(ids.contains(&"a") && ids.contains(&"c"));
        // Best score normalised to 1.0; everything stays in (0, 1], descending.
        assert!((fused[0].score - 1.0).abs() < 1e-6);
        assert!(fused.iter().all(|s| s.score > 0.0 && s.score <= 1.0));
        for pair in fused.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
        // Dense best-chunk index preserved; sparse-only hit carries 0.
        assert_eq!(
            fused.iter().find(|s| s.mail_id == "b").unwrap().chunk_index,
            1
        );
        assert_eq!(
            fused.iter().find(|s| s.mail_id == "c").unwrap().chunk_index,
            0
        );
    }

    #[test]
    fn fuse_rrf_empty_inputs_yield_empty() {
        assert!(fuse_rrf(Vec::new(), Vec::new(), 10).is_empty());
    }

    #[tokio::test]
    async fn hybrid_surfaces_keyword_only_mail() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        // A topically-related mail WITH a vector — the dense leg can find it.
        seed_mail(
            &state,
            "vec",
            "a",
            None,
            "peer@x.com",
            "quarterly budget revenue report",
            now_unix(),
        )
        .await;
        index_mail(&state, "vec", "a", &["quarterly budget revenue report"]).await;
        // A keyword-only mail with NO vector — the dense leg can never return it,
        // so its presence proves the sparse BM25 leg is doing the work.
        seed_mail(
            &state,
            "kw",
            "a",
            None,
            "peer@x.com",
            "the zephyrite ledger reconciliation",
            now_unix() - 50,
        )
        .await;

        let ranked = retrieve_scored(
            &state,
            "a",
            "zephyrite reconciliation",
            None,
            10,
            DEFAULT_MIN_SCORE,
        )
        .await
        .unwrap();
        let ids: Vec<&str> = ranked.iter().map(|s| s.mail_id.as_str()).collect();
        assert!(
            ids.contains(&"kw"),
            "BM25 leg surfaces a keyword-only mail the vector index misses"
        );
    }

    #[tokio::test]
    async fn role_preamble_reflects_role_type_and_description() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(
            &state,
            "a",
            "sales",
            Some("Qualify inbound leads and quote pricing."),
        )
        .await;
        seed_mail(&state, "m", "a", None, "peer@x.com", "hello", now_unix()).await;

        let ctx = assemble_role_context(&state, &params("m", "a", 100_000))
            .await
            .unwrap();
        assert!(ctx.role_preamble.contains("sales"));
        assert!(ctx.role_preamble.contains("a@x.com"));
        assert!(ctx
            .role_preamble
            .contains("Qualify inbound leads and quote pricing."));
        assert!(ctx.system_preamble().contains(SAFETY_PREAMBLE));
    }

    #[tokio::test]
    async fn thread_mails_capped_at_five_excluding_trigger() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_thread(&state, "t1", "a").await;
        let base = now_unix();
        seed_mail(
            &state,
            "trigger",
            "a",
            Some("t1"),
            "peer@x.com",
            "latest ask",
            base,
        )
        .await;
        for i in 0..8 {
            seed_mail(
                &state,
                &format!("tm{i}"),
                "a",
                Some("t1"),
                "peer@x.com",
                "earlier reply in the thread",
                base - 10 - i as i64,
            )
            .await;
        }

        let mut p = params("trigger", "a", 100_000);
        p.thread_id = Some("t1".into());
        let ctx = assemble_role_context(&state, &p).await.unwrap();

        assert_eq!(ctx.thread_mails.len(), 5);
        assert!(ctx.thread_mails.iter().all(|m| m.mail_id != "trigger"));
        // Newest first.
        for pair in ctx.thread_mails.windows(2) {
            assert!(pair[0].date_sent >= pair[1].date_sent);
        }
    }

    #[tokio::test]
    async fn contact_history_present_and_correct() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "sales", None).await;
        seed_mail(
            &state,
            "m",
            "a",
            None,
            "client@corp.com",
            "pricing question",
            now_unix(),
        )
        .await;
        seed_contact(&state, "client@corp.com", 17, 9).await;

        let ctx = assemble_role_context(&state, &params("m", "a", 100_000))
            .await
            .unwrap();
        let history = ctx.contact_history.expect("contact row exists");
        assert_eq!(history.email, "client@corp.com");
        assert_eq!(history.data.interaction_count, 17);
        assert_eq!(history.data.reply_count, 9);
    }

    #[tokio::test]
    async fn missing_contact_returns_none() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_mail(
            &state,
            "m",
            "a",
            None,
            "stranger@new.com",
            "first contact",
            now_unix(),
        )
        .await;

        let ctx = assemble_role_context(&state, &params("m", "a", 100_000))
            .await
            .unwrap();
        assert!(ctx.contact_history.is_none());
    }

    #[tokio::test]
    async fn trigger_mail_excluded_from_chunks() {
        let (state, _rx) = AppState::test_state().await;
        seed_corpus(&state, "a").await;

        let ctx = assemble_role_context(&state, &params("trigger", "a", 100_000))
            .await
            .unwrap();
        assert!(
            ctx.chunks.iter().all(|c| c.mail_id != "trigger"),
            "the trigger mail must never ground itself"
        );
        assert!(ctx.knowledge_refs.iter().all(|id| id != "trigger"));
    }

    #[tokio::test]
    async fn duplicate_chunks_of_same_mail_dedup_in_refs() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "legal", None).await;
        seed_mail(
            &state,
            "trigger",
            "a",
            None,
            "peer@x.com",
            TRIGGER_BODY,
            now_unix(),
        )
        .await;
        // One prior mail indexed as TWO chunks, both close to the query.
        seed_mail(
            &state,
            "multi",
            "a",
            None,
            "peer@x.com",
            TRIGGER_BODY,
            now_unix() - 50,
        )
        .await;
        index_mail(
            &state,
            "multi",
            "a",
            &[
                "licensing contract renewal terms and the indemnity clause",
                "quarterly licensing contract renewal indemnity review",
            ],
        )
        .await;

        let ctx = assemble_role_context(&state, &params("trigger", "a", 100_000))
            .await
            .unwrap();
        assert_eq!(
            ctx.knowledge_refs
                .iter()
                .filter(|id| *id == "multi")
                .count(),
            1,
            "one ref per mail no matter how many chunks matched"
        );
        assert_eq!(
            ctx.chunks.iter().filter(|c| c.mail_id == "multi").count(),
            1,
            "per-mail aggregation keeps only the best chunk"
        );
    }

    #[tokio::test]
    async fn tight_budget_drops_chunks_before_thread() {
        let (state, _rx) = AppState::test_state().await;
        seed_corpus(&state, "a").await;
        seed_thread(&state, "t1", "a").await;
        sqlx::query("UPDATE mails SET thread_id = 't1' WHERE id = 'trigger'")
            .execute(state.storage.db().pool())
            .await
            .unwrap();
        seed_mail(
            &state,
            "tm1",
            "a",
            Some("t1"),
            "peer@x.com",
            "short thread reply",
            now_unix() - 5,
        )
        .await;

        let mut p = params("trigger", "a", 100_000);
        p.thread_id = Some("t1".into());
        let full = assemble_role_context(&state, &p).await.unwrap();
        assert!(!full.chunks.is_empty());
        assert_eq!(full.thread_mails.len(), 1);

        // One token under the full cost: the lowest-priority item (a GTE chunk)
        // is dropped first; the thread snippet survives.
        p.token_budget = full.total_tokens_used - 1;
        let trimmed = assemble_role_context(&state, &p).await.unwrap();
        assert_eq!(trimmed.thread_mails.len(), full.thread_mails.len());
        assert!(trimmed.chunks.len() < full.chunks.len());
        assert_eq!(trimmed.knowledge_refs.len(), trimmed.chunks.len());
        assert!(trimmed.total_tokens_used <= p.token_budget);
    }

    #[tokio::test]
    async fn minimum_fit_budget_returns_ok_with_no_extras() {
        let (state, _rx) = AppState::test_state().await;
        seed_corpus(&state, "a").await;

        // Budget exactly covers preambles + target mail — nothing else fits,
        // but the call must succeed (acceptance: tiny budget never panics).
        let role = build_role_preamble(
            "a@x.com",
            "legal",
            Some("Review inbound contracts for risk."),
        );
        let minimum = estimate_tokens(SAFETY_PREAMBLE)
            + estimate_tokens(&role)
            + estimate_tokens("Subject trigger")
            + estimate_tokens(TRIGGER_BODY);
        let ctx = assemble_role_context(&state, &params("trigger", "a", minimum))
            .await
            .unwrap();
        assert!(ctx.chunks.is_empty());
        assert!(ctx.thread_mails.is_empty());
        assert!(ctx.knowledge_refs.is_empty());
        assert_eq!(ctx.total_tokens_used, minimum);
    }

    #[tokio::test]
    async fn target_mail_exceeding_budget_is_context_too_long() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "legal", None).await;
        let huge = "contract clause ".repeat(2_000); // ~8000 tokens
        seed_mail(&state, "big", "a", None, "peer@x.com", &huge, now_unix()).await;

        let err = assemble_role_context(&state, &params("big", "a", 50))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::AiContextTooLong));
    }

    #[tokio::test]
    async fn unknown_mail_or_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_mail(&state, "m", "a", None, "peer@x.com", "hello", now_unix()).await;

        let err = assemble_role_context(&state, &params("missing", "a", 1_000))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound));

        // A mail id that exists but under another account must not leak.
        let err = assemble_role_context(&state, &params("m", "other", 1_000))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound));
    }

    #[test]
    fn token_estimate_is_conservative_and_nonzero() {
        assert_eq!(estimate_tokens(""), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
    }

    #[test]
    fn chunk_index_parses_from_chunk_id() {
        assert_eq!(chunk_index_of("mail-uuid:3"), 3);
        assert_eq!(chunk_index_of("mail-uuid:0"), 0);
        assert_eq!(chunk_index_of("garbage"), 0);
    }
}
