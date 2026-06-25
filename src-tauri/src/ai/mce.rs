//! Mailbox Context Engine (MCE) — the shared retrieval + assembly core
//! (analysis/54, phase P-1).
//!
//! Historically two AI behaviours fetched local mail through two *different*
//! code paths: the reply/analysis path went through the rich, budget-managed,
//! audited [`assemble_role_context`](super::context::assemble_role_context)
//! (T074), while the TEAM-channel chat path ran a thin, ad-hoc semantic search
//! with no budget, no provenance, and a silent "no data" failure. Two fetch
//! capabilities, only one of them good — the N7 "duplicate-implementation"
//! hazard the repo warns about.
//!
//! The MCE folds both into one engine. P-1 ships the *skeleton*: a uniform
//! retrieved unit ([`ContextItem`]), an honest provenance report
//! ([`RetrievalReport`]), one assembled output shape ([`AssembledContext`]), and
//! a single semantic retrieval implementation (leg B, in
//! [`super::context::retrieve_scored`]) shared by both entry points:
//!
//! * [`MailboxContextEngine::assemble_for_question`] — the anchorless chat path.
//!   A free-text question becomes the semantic query over the whole account;
//!   results are budget-packed and reported with honest state.
//! * [`MailboxContextEngine::assemble_for_mail`] — the anchored reply/analysis
//!   path, a thin adapter over the proven `assemble_role_context`, mapped into
//!   the same [`AssembledContext`] so new callers share one shape.
//!
//! Retrieval legs C–F (aggregate SQL, sender, temporal, memory) and the planner
//! (intent routing / tool-use) are later phases (P-2…P-5); they slot in behind
//! this same contract without touching callers.
//!
//! **Read/act boundary (analysis/54 §3.6):** the MCE only *reads & understands*
//! (retrieve + assemble). It never sends mail; the digital employee's AI Reply
//! Mode (Full / Semi / Manual) still gates only the write/send step.
//!
//! **Logging red-line (dev/09 §5):** identifiers and counts only — never
//! `body_text`, snippets, subjects, or addresses.

use std::collections::HashMap;

use serde::Deserialize;
use sqlx::Row;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::{inbox_digest_repo, thread_summary_repo};
use crate::util::{now_unix, truncate_chars};

use super::context::{
    self, ContactHistory, RoleContext, RoleContextParams, ScoredMail, DEFAULT_MIN_SCORE,
    SAFETY_PREAMBLE,
};
use super::types::{Capability, ChatMessage, ChatRequest, ChatRole};

/// Default semantic fan-out for the question path. Matches the chat path's prior
/// hit count so routing through the engine is not a silent behaviour change.
pub const DEFAULT_QUESTION_TOP_K: usize = 6;

/// Which retrieval "leg" produced a context item (analysis/54 §3.2). P-1 shipped
/// `Target`/`Thread` (anchored, leg A) and `Semantic` (leg B); P-2 added
/// `Temporal` (leg E) and `Aggregate` (leg C); P-4 added `Memory` (leg F) and
/// `Sender` (leg D) — all behind this same contract, no caller changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextItemKind {
    /// The anchor mail itself (anchored requests only).
    Target,
    /// A same-thread neighbour of the anchor (anchored requests only).
    Thread,
    /// A GTE/ANN semantic hit over the account.
    Semantic,
    /// A mail selected by recency (leg E): newest-N within an optional time
    /// window, ordered by arrival, not similarity.
    Temporal,
    /// A computed structured fact (leg C): a count, an unread total, a
    /// top-senders ranking. Has no single source mail, so `mail_id` is empty.
    Aggregate,
    /// A precomputed per-thread summary (leg F, P-4): the map-reduce shortcut
    /// that lets "summarise everything" read one line per thread instead of
    /// every raw mail. Synthesised over a thread, so `mail_id` is empty.
    Memory,
    /// A counterparty's profile from `contacts` (leg D, analysis/54 §3.2): the
    /// interaction stats / trust / style of a person or company the question
    /// names. Synthesised over a contact, so `mail_id` is empty.
    Sender,
}

/// One uniform, citable unit of retrieved context. Every leg returns these, so
/// budget packing, citation (`knowledge_refs`), and audit are identical across
/// legs and across the reply / chat / analysis callers.
#[derive(Debug, Clone)]
pub struct ContextItem {
    pub kind: ContextItemKind,
    /// Source mail id — the provenance every item carries.
    pub mail_id: String,
    /// Subject, when hydrated. Empty for anchored semantic items, whose source
    /// `KnowledgeChunk` carries only the snippet (the anchored prompt renders
    /// them by snippet alone).
    pub subject: String,
    /// Sender address, when hydrated (see `subject`).
    pub from_email: String,
    pub date_sent: i64,
    /// Body (target / thread) or 200-char snippet (semantic), already
    /// B1-sanitised upstream.
    pub content: String,
    /// Cosine score for semantic hits; `None` for anchored target/thread items.
    pub score: Option<f32>,
}

/// A free-text question over one account's mailbox — the chat path's request.
/// There is no anchor mail; the `query` text itself is the semantic query.
#[derive(Debug, Clone)]
pub struct QuestionParams {
    /// The operator's natural-language question.
    pub query: String,
    /// The agent (digital employee) answering — scopes retrieval to its mailbox.
    pub account_id: String,
    /// Token budget for the assembled mail context (typically the same
    /// thread-context share of the model window the reply path uses).
    pub token_budget: usize,
    /// Semantic fan-out after per-mail aggregation.
    pub top_k: usize,
    /// Cosine floor below which semantic hits are discarded.
    pub min_score: f32,
    /// What the context is for; recorded for audit/logging.
    pub capability: Capability,
    /// Opt in to the P-5 slow path: when the deterministic router finds nothing
    /// (a non-keyword or non-English phrasing), let the model classify the
    /// question into the legs. Off by default — only callers willing to spend one
    /// extra small model call (e.g. interactive chat) enable it.
    pub allow_model_planner: bool,
}

impl QuestionParams {
    /// Params with the question-path defaults (`top_k = 6`, `min_score = 0.35`,
    /// fast path only).
    pub fn new(
        query: impl Into<String>,
        account_id: impl Into<String>,
        token_budget: usize,
        capability: Capability,
    ) -> Self {
        Self {
            query: query.into(),
            account_id: account_id.into(),
            token_budget,
            top_k: DEFAULT_QUESTION_TOP_K,
            min_score: DEFAULT_MIN_SCORE,
            capability,
            allow_model_planner: false,
        }
    }
}

/// Honest provenance / state for one assembly (analysis/54 §3.4). P-1 seeds it;
/// P-3 surfaces it in the UI so the agent can say *why* mail context is thin
/// ("index unavailable" vs "no matches") instead of a silent empty answer.
#[derive(Debug, Clone, Default)]
pub struct RetrievalReport {
    /// Whether this assembly was anchored on a specific mail.
    pub anchored: bool,
    /// Semantic items kept after budget packing.
    pub semantic_hits: usize,
    /// Same-thread items kept (anchored path only).
    pub thread_mails: usize,
    /// Recency-selected items kept (leg E).
    pub temporal_hits: usize,
    /// Computed structured facts kept (leg C).
    pub aggregate_facts: usize,
    /// Precomputed thread summaries kept (leg F).
    pub memory_hits: usize,
    /// Counterparty profiles kept (leg D).
    pub sender_hits: usize,
    /// `false` when the semantic index/embedder was unavailable, so callers can
    /// distinguish "the index couldn't run" from "the index ran and found
    /// nothing." The deliberate opposite of the old silent swallow.
    pub semantic_available: bool,
    /// Mails in this account already embedded into the semantic index
    /// (`embedding_status = 'indexed'`). With `total_mails`, this is the index
    /// coverage the UI surfaces so a partial index reads as "searched N of M"
    /// rather than a misleading empty answer (analysis/54 §3.4). The question
    /// path populates these; the anchored adapter leaves them `0`.
    pub indexed_mails: usize,
    /// Stored, non-deleted mails in this account — the denominator for coverage.
    pub total_mails: usize,
}

/// The assembled, budget-fitted context bundle — the single shape every AI
/// behaviour consumes (analysis/54 §3, §4).
#[derive(Debug, Clone)]
pub struct AssembledContext {
    /// Persona line from the account's role columns.
    pub role_preamble: String,
    /// The fixed safety contract ([`SAFETY_PREAMBLE`]).
    pub safety_preamble: String,
    /// Retrieved items in packed (priority) order: target → thread → semantic.
    pub items: Vec<ContextItem>,
    /// Deduplicated source `mail_id`s for the `ai_drafts` / `ai_decisions`
    /// `knowledge_refs` audit columns.
    pub knowledge_refs: Vec<String>,
    /// Counterparty stats (anchored analysis paths); `None` for the question
    /// path, which has no single counterparty.
    pub contact_history: Option<ContactHistory>,
    /// Provenance / honest state for this assembly.
    pub report: RetrievalReport,
    /// Estimated tokens of everything kept; always `<= token_budget`.
    pub total_tokens_used: usize,
}

impl AssembledContext {
    /// Role + safety preamble joined for `ChatRequest::system` (dev/06 §5).
    pub fn system_preamble(&self) -> String {
        format!("{}\n\n{}", self.role_preamble, self.safety_preamble)
    }

    /// Items from one leg, in packed order.
    pub fn items_of(&self, kind: ContextItemKind) -> impl Iterator<Item = &ContextItem> {
        self.items.iter().filter(move |item| item.kind == kind)
    }

    /// The anchor mail (anchored requests). Always present for
    /// [`MailboxContextEngine::assemble_for_mail`]; `None` for question requests.
    pub fn target(&self) -> Option<&ContextItem> {
        self.items
            .iter()
            .find(|item| item.kind == ContextItemKind::Target)
    }
}

/// The shared engine. Cheap to construct (`new`); holds only a borrowed
/// [`AppState`].
pub struct MailboxContextEngine<'a> {
    state: &'a AppState,
}

impl<'a> MailboxContextEngine<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    /// Anchorless question path (chat). A fast-path planner ([`plan_question`])
    /// routes the question to retrieval legs, which are packed to `token_budget`
    /// in priority order (aggregate facts → recent mails → semantic hits) and
    /// reported with honest state.
    ///
    /// * Leg C (aggregate, SQL): counts / unread total / top senders — the
    ///   direct fix for "how many unread", "who emails me most".
    /// * Leg E (temporal, SQL): newest-N within a time window — "what's new",
    ///   "today", "this week".
    /// * Leg B (semantic): the fallback for topic/person questions, unchanged
    ///   from P-1.
    ///
    /// Semantic-index failure is **not** fatal: the bundle still returns with
    /// `report.semantic_available = false`, so the agent can answer from the
    /// conversation and say it couldn't search (analysis/54 §3.4) — the opposite
    /// of the old silent swallow. A missing account or a DB fault still
    /// propagates.
    pub async fn assemble_for_question(&self, p: &QuestionParams) -> AppResult<AssembledContext> {
        let state = self.state;
        let role_preamble = role_preamble_for_account(state, &p.account_id).await?;
        let safety_preamble = SAFETY_PREAMBLE.to_string();

        // Fast-path planner (analysis/54 §3.1): deterministic keyword routing
        // picks the legs, no model call.
        let mut plan = plan_question(&p.query);
        // Slow path (P-5): when the deterministic router found nothing (the
        // semantic catch-all) and the caller opted in, let the model classify the
        // question into the legs — paraphrase- and non-English-robust routing.
        // Any failure quietly keeps the fast-path plan.
        if p.allow_model_planner && plan.semantic {
            if let Some(model_plan) = plan_question_llm(state, &p.account_id, &p.query).await {
                plan = model_plan;
            }
        }

        let mut used =
            context::estimate_tokens(&role_preamble) + context::estimate_tokens(&safety_preamble);
        let mut items: Vec<ContextItem> = Vec::new();

        // Leg D — the named counterparty's profile, when the question is about a
        // specific person or company (an email address or a "history with X"
        // phrase). Tiny and highly specific, so it leads the bundle.
        let mut sender_hits = 0usize;
        if let Some(sender) = detect_sender(&p.query) {
            for item in retrieve_sender(state, &sender, p.top_k).await? {
                let cost = context::estimate_tokens(&item.content);
                if used + cost > p.token_budget {
                    break;
                }
                used += cost;
                sender_hits += 1;
                items.push(item);
            }
        }

        // Leg C — aggregate facts first: a count answers "how many unread"
        // outright, and the facts are tiny, so they earn top priority.
        let mut aggregate_facts = 0usize;
        for spec in &plan.aggregates {
            if let Some(item) = compute_aggregate(state, &p.account_id, *spec).await? {
                let cost = context::estimate_tokens(&item.content);
                if used + cost > p.token_budget {
                    break;
                }
                used += cost;
                aggregate_facts += 1;
                items.push(item);
            }
        }

        // Leg F — precomputed memory (P-4). For "summarise everything" questions
        // the rolling inbox digest (level-2 reduction) leads — one paragraph that
        // stands in for the whole mailbox even when per-thread summaries don't all
        // fit — followed by as many per-thread summaries as the budget allows.
        let mut memory_hits = 0usize;
        if plan.memory {
            let digest = retrieve_digest(state, &p.account_id).await?;
            let memory_items = digest
                .into_iter()
                .chain(retrieve_memory(state, &p.account_id, p.top_k).await?);
            for item in memory_items {
                let cost = context::estimate_tokens(&item.content);
                if used + cost > p.token_budget {
                    break;
                }
                used += cost;
                memory_hits += 1;
                items.push(item);
            }
        }

        // Leg E — temporal newest-N within the planned window. Runs when the
        // planner chose it directly, or as the fallback when the memory leg was
        // wanted but no summaries exist yet (so overview still works on day one).
        let mut temporal_hits = 0usize;
        if plan.temporal.is_some() && (!plan.memory || memory_hits == 0) {
            if let Some(temporal) = plan.temporal {
                for item in retrieve_temporal(state, &p.account_id, temporal, p.top_k).await? {
                    let cost = context::estimate_tokens(&item.subject)
                        + context::estimate_tokens(&item.content);
                    if used + cost > p.token_budget {
                        break;
                    }
                    used += cost;
                    temporal_hits += 1;
                    items.push(item);
                }
            }
        }

        // Leg B — semantic retrieval: when the planner routed here (a topic or
        // person question). Index/embedder errors degrade to "no context, but
        // say so".
        let run_semantic = plan.semantic;
        let mut semantic_hits = 0usize;
        let mut semantic_available = true;
        if run_semantic {
            match context::retrieve_scored(
                state,
                &p.account_id,
                &p.query,
                None,
                p.top_k,
                p.min_score,
            )
            .await
            {
                Ok(scored) => {
                    for item in hydrate_semantic_items(state, scored).await? {
                        let cost = context::estimate_tokens(&item.subject)
                            + context::estimate_tokens(&item.content);
                        if used + cost > p.token_budget {
                            break;
                        }
                        used += cost;
                        semantic_hits += 1;
                        items.push(item);
                    }
                }
                Err(err) => {
                    tracing::debug!(
                        event = "mce_question_semantic_unavailable",
                        account_id = %p.account_id,
                        code = err.code().as_wire(),
                        "semantic leg unavailable; answering without mail context"
                    );
                    semantic_available = false;
                }
            }
        }

        // Index coverage for honest state — how much of this mailbox the
        // semantic index can actually see (analysis/54 §3.4). Cheap counts; the
        // UI turns a partial index into "searched N of M" instead of "no data".
        let (indexed_mails, total_mails) = index_coverage(state, &p.account_id).await?;

        let knowledge_refs = dedup_refs(&items);
        let report = RetrievalReport {
            anchored: false,
            semantic_hits,
            thread_mails: 0,
            temporal_hits,
            aggregate_facts,
            memory_hits,
            sender_hits,
            semantic_available,
            indexed_mails,
            total_mails,
        };

        tracing::info!(
            event = "mce_question_assembled",
            account_id = %p.account_id,
            capability = p.capability.as_str(),
            semantic_hits,
            temporal_hits,
            aggregate_facts,
            memory_hits,
            sender_hits,
            semantic_available,
            indexed_mails,
            total_mails,
            tokens = used,
            budget = p.token_budget,
            "assembled question context"
        );

        Ok(AssembledContext {
            role_preamble,
            safety_preamble,
            items,
            knowledge_refs,
            contact_history: None,
            report,
            total_tokens_used: used,
        })
    }

    /// Anchored path (reply / analysis). Wraps the proven
    /// [`assemble_role_context`](super::context::assemble_role_context), maps it
    /// into the unified [`AssembledContext`], then tops the bundle up with the
    /// precomputed **memory leg** (analysis/55 §4, Gap A): this thread's summary
    /// plus the rolling inbox digest, packed into whatever token budget the
    /// anchored legs left. Drafts and D1 / D2 analysis now see the conversation's
    /// long-term arc and the wider mailbox state — not just the target mail, its
    /// thread, and semantic hits. Legacy callers that still invoke
    /// `assemble_role_context` directly gain memory only once they migrate to this
    /// front door.
    pub async fn assemble_for_mail(&self, p: &RoleContextParams) -> AppResult<AssembledContext> {
        let ctx = context::assemble_role_context(self.state, p).await?;
        let mut assembled = from_role_context(ctx);
        append_anchored_memory(self.state, p, &mut assembled).await?;
        Ok(assembled)
    }
}

/// Build the persona line for an account the same way the anchored path does
/// (dev/06 §5 item 1). `NotFound` when the account row is gone.
async fn role_preamble_for_account(state: &AppState, account_id: &str) -> AppResult<String> {
    let row = sqlx::query("SELECT email, role_type, role_description FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(state.storage.db().pool())
        .await
        .map_err(crate::storage::map_sqlx_err)?
        .ok_or(AppError::NotFound)?;
    Ok(context::build_role_preamble(
        &row.get::<String, _>("email"),
        &row.get::<String, _>("role_type"),
        row.get::<Option<String>, _>("role_description").as_deref(),
    ))
}

/// Hydrate ranked semantic hits into citable [`ContextItem`]s (subject, sender,
/// date, 200-char snippet), preserving the score-descending order.
async fn hydrate_semantic_items(
    state: &AppState,
    ranked: Vec<ScoredMail>,
) -> AppResult<Vec<ContextItem>> {
    if ranked.is_empty() {
        return Ok(Vec::new());
    }
    let db = state.storage.db().pool();
    let placeholders = vec!["?"; ranked.len()].join(",");
    let sql = format!(
        "SELECT id, subject, from_email, date_sent, COALESCE(snippet, '') AS snippet \
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
    let mut meta: HashMap<String, (String, String, i64, String)> = rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("id"),
                (
                    r.get::<String, _>("subject"),
                    r.get::<String, _>("from_email"),
                    r.get::<i64, _>("date_sent"),
                    r.get::<String, _>("snippet"),
                ),
            )
        })
        .collect();

    Ok(ranked
        .into_iter()
        .filter_map(|sm| {
            meta.remove(&sm.mail_id)
                .map(|(subject, from_email, date_sent, snippet)| ContextItem {
                    kind: ContextItemKind::Semantic,
                    mail_id: sm.mail_id,
                    subject,
                    from_email,
                    date_sent,
                    content: truncate_chars(&snippet, context::CHUNK_SNIPPET_CHARS),
                    score: Some(sm.score),
                })
        })
        .collect())
}

/// Source `mail_id`s, deduplicated in first-seen (priority) order. Aggregate
/// facts carry no source mail, so their empty `mail_id` is skipped — only real
/// emails become citable `knowledge_refs`.
fn dedup_refs(items: &[ContextItem]) -> Vec<String> {
    let mut refs: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        if item.mail_id.is_empty() {
            continue;
        }
        if !refs.iter().any(|id| id == &item.mail_id) {
            refs.push(item.mail_id.clone());
        }
    }
    refs
}

/// Map the anchored [`RoleContext`] into the unified [`AssembledContext`]:
/// target first, then thread members, then semantic chunks — the packer's
/// priority order. The `knowledge_refs`, contact history, preambles, and token
/// total pass through unchanged.
fn from_role_context(ctx: RoleContext) -> AssembledContext {
    let RoleContext {
        role_preamble,
        safety_preamble,
        target_mail,
        thread_mails,
        chunks,
        knowledge_refs,
        contact_history,
        total_tokens_used,
    } = ctx;

    let mut items = Vec::with_capacity(1 + thread_mails.len() + chunks.len());
    items.push(ContextItem {
        kind: ContextItemKind::Target,
        mail_id: target_mail.mail_id,
        subject: target_mail.subject,
        from_email: target_mail.from_email,
        date_sent: target_mail.date_sent,
        content: target_mail.body,
        score: None,
    });
    let thread_count = thread_mails.len();
    for mail in thread_mails {
        items.push(ContextItem {
            kind: ContextItemKind::Thread,
            mail_id: mail.mail_id,
            subject: mail.subject,
            from_email: mail.from_email,
            date_sent: mail.date_sent,
            content: mail.body,
            score: None,
        });
    }
    let semantic_count = chunks.len();
    for chunk in chunks {
        items.push(ContextItem {
            kind: ContextItemKind::Semantic,
            mail_id: chunk.mail_id,
            // Anchored chunks carry only the snippet; the anchored prompt renders
            // them by snippet alone, so subject/sender stay empty here.
            subject: String::new(),
            from_email: String::new(),
            date_sent: chunk.date_sent,
            content: chunk.snippet,
            score: Some(chunk.score),
        });
    }

    AssembledContext {
        role_preamble,
        safety_preamble,
        items,
        knowledge_refs,
        contact_history,
        report: RetrievalReport {
            anchored: true,
            semantic_hits: semantic_count,
            thread_mails: thread_count,
            temporal_hits: 0,
            aggregate_facts: 0,
            memory_hits: 0,
            // Anchored callers read the counterparty via `contact_history`, not a
            // Sender leg item.
            sender_hits: 0,
            semantic_available: true,
            // Coverage is a question-path concern (surfaced in chat); the
            // anchored reply/analysis path leaves it unset.
            indexed_mails: 0,
            total_mails: 0,
        },
        total_tokens_used,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Fast-path planner (analysis/54 §3.1) and the aggregate/temporal legs (§3.2).
// ─────────────────────────────────────────────────────────────────────────────

/// A time window for the temporal leg. UTC-anchored: "today" is since 00:00 UTC,
/// "this week" is a rolling 7-day window. A backend-side approximation — the
/// device's local timezone is not known here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeWindow {
    /// No lower bound — just the newest mails overall.
    AllTime,
    /// Arrived since 00:00 UTC today.
    Today,
    /// Arrived within the last 7 days.
    ThisWeek,
}

/// The temporal leg's parameters: a window plus an optional unread-only scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalQuery {
    pub window: TimeWindow,
    pub unread_only: bool,
}

/// One computed fact the aggregate leg can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateQuery {
    /// Number of mails in the inbox.
    TotalCount,
    /// Number of unread mails in the inbox.
    UnreadCount,
    /// The most frequent senders, by mail count.
    TopSenders,
}

/// The fast-path plan: which legs to run for a question, chosen by deterministic
/// keyword routing. The aggregate and temporal legs cover the
/// "summarise / count / unread / recent" family the chat path used to fail;
/// `semantic` is the fallback for topic/person questions (analysis/54 §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPlan {
    pub semantic: bool,
    /// Prefer the precomputed thread-summary memory leg (leg F) over dumping raw
    /// recent mail — set for "summarise everything" style questions.
    pub memory: bool,
    pub temporal: Option<TemporalQuery>,
    pub aggregates: Vec<AggregateQuery>,
}

/// Route a free-text question to retrieval legs (analysis/54 §3.1 fast path).
/// Pure and deterministic — cheap to run, easy to test, and a safe fallback even
/// once the P-5 slow path (model tool-calling) lands. Obvious aggregate/temporal
/// words route to legs C/E; anything else falls through to semantic (leg B).
pub fn plan_question(query: &str) -> QueryPlan {
    let q = query.to_lowercase();
    let has_any = |needles: &[&str]| needles.iter().any(|n| q.contains(n));

    let unread_only = has_any(&[
        "unread",
        "unanswered",
        "not read",
        "haven't read",
        "havent read",
    ]);
    let overview = has_any(&[
        "summarize",
        "summarise",
        "summary",
        "overview",
        "recap",
        "catch me up",
        "catch up",
        "digest",
        "what's in my inbox",
        "whats in my inbox",
        "go through my inbox",
    ]);
    let counting = has_any(&[
        "how many",
        "how much",
        "number of",
        "count of",
        "total number",
    ]);
    let ranking = has_any(&[
        "top sender",
        "by sender",
        "most email",
        "most message",
        "most frequent",
        "busiest",
        "from whom",
        "which sender",
        "who emails",
        "who sends",
        "who messages",
        "who's emailing",
        "whos emailing",
    ]);

    let window = if has_any(&["today", "this morning", "so far today"]) {
        Some(TimeWindow::Today)
    } else if has_any(&[
        "this week",
        "past week",
        "last week",
        "last 7 days",
        "this month",
    ]) {
        Some(TimeWindow::ThisWeek)
    } else {
        None
    };
    let recency = has_any(&[
        "recent",
        "lately",
        "latest",
        "newest",
        "new email",
        "new message",
        "what's new",
        "whats new",
        "anything new",
        "just came in",
        "just arrived",
    ]);
    let temporal = (window.is_some() || recency || overview).then(|| TemporalQuery {
        window: window.unwrap_or(TimeWindow::AllTime),
        unread_only,
    });

    let mut aggregates = Vec::new();
    if overview {
        aggregates.push(AggregateQuery::TotalCount);
        aggregates.push(AggregateQuery::UnreadCount);
        aggregates.push(AggregateQuery::TopSenders);
    } else {
        if unread_only {
            aggregates.push(AggregateQuery::UnreadCount);
        } else if counting {
            aggregates.push(AggregateQuery::TotalCount);
        }
        if ranking {
            aggregates.push(AggregateQuery::TopSenders);
        }
    }

    // Overview questions prefer the precomputed thread-summary memory leg; the
    // temporal leg stays as the fallback when no summaries exist yet.
    let memory = overview;

    // Semantic is the fallback for topic/person questions — only when no
    // deterministic memory/aggregate/temporal signal fired.
    let semantic = !memory && aggregates.is_empty() && temporal.is_none();

    QueryPlan {
        semantic,
        memory,
        temporal,
        aggregates,
    }
}

/// A counterparty the question is asking about (leg D): an explicit email address
/// or a name/organisation keyword to match against `contacts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SenderQuery {
    /// An explicit address found in the question (lowercased), matched exactly.
    pub email: Option<String>,
    /// A name/organisation keyword, LIKE-matched when no email is present.
    pub name: Option<String>,
}

/// Relationship-intent phrases that introduce a named counterparty.
const SENDER_TRIGGERS: &[&str] = &[
    "history with ",
    "relationship with ",
    "dealings with ",
    "correspondence with ",
    "interactions with ",
    "exchanges with ",
    "dealt with ",
    "what do i know about ",
    "what do you know about ",
    "tell me about ",
];

/// Detect a counterparty the question is about (leg D, analysis/54 §3.2). Prefers
/// an explicit email address; otherwise, when a relationship-intent phrase is
/// present, takes the trailing words as a name/organisation to match. Pure and
/// deterministic. `None` when the question names no counterparty.
pub(crate) fn detect_sender(query: &str) -> Option<SenderQuery> {
    if let Some(email) = find_email(query) {
        return Some(SenderQuery {
            email: Some(email),
            name: None,
        });
    }
    let lower = query.to_lowercase();
    for trigger in SENDER_TRIGGERS {
        if let Some(pos) = lower.find(trigger) {
            if let Some(name) = clean_name(&lower[pos + trigger.len()..]) {
                return Some(SenderQuery {
                    email: None,
                    name: Some(name),
                });
            }
        }
    }
    None
}

/// First email-looking token in `q`, lowercased, or `None`. Hand-rolled (no regex
/// dependency): expands an alphanumeric/`._%+-` local part left of an `@` and an
/// alphanumeric/`.-` domain right of it, requiring a dotted domain.
fn find_email(q: &str) -> Option<String> {
    let bytes = q.as_bytes();
    let at = q.find('@')?;
    let is_local =
        |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-');
    let is_domain = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-');
    let mut start = at;
    while start > 0 && is_local(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < bytes.len() && is_domain(bytes[end]) {
        end += 1;
    }
    if start == at || end == at + 1 {
        return None; // empty local part or domain
    }
    let domain = &q[at + 1..end];
    if !domain.contains('.') || domain.ends_with('.') {
        return None;
    }
    Some(q[start..end].to_lowercase())
}

/// The first ≤4 words of `tail`, up to sentence-ending punctuation, as a name
/// keyword. `None` when nothing usable remains.
fn clean_name(tail: &str) -> Option<String> {
    let head = tail
        .split(['.', '?', '!', ',', ';', ':', '\n'])
        .next()
        .unwrap_or("");
    let name = head
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    if name.chars().count() < 2 {
        None
    } else {
        Some(name)
    }
}

/// Output token ceiling for the slow-path planner call.
const PLANNER_MAX_TOKENS: u32 = 120;

const PLANNER_SYSTEM: &str = "You route a mailbox question to retrieval tools. Reply with STRICT \
JSON only — no prose, no code fence:\n\
{\"intent\": \"overview|recent|count|unread|senders|topic\", \"window\": \"all|today|week\", \
\"unread_only\": true|false}\n\
Definitions: overview = summarise the whole inbox; recent = the latest mail; count = how many \
emails; unread = unread mail; senders = who emails most; topic = about a specific subject or \
person. Choose the single best intent. The question may be in any language.";

#[derive(Debug, Deserialize)]
struct PlanJson {
    intent: String,
    #[serde(default)]
    window: String,
    #[serde(default)]
    unread_only: bool,
}

/// Slow-path planner (analysis/54 §3.1, P-5): ask the model to classify the
/// question into one intent + window, then build the same [`QueryPlan`] the legs
/// consume. Provider-gated and provider-agnostic (plain JSON, not vendor
/// function-calling). Returns `None` on no provider, model error, or unparsable
/// output — the caller then keeps the deterministic plan.
async fn plan_question_llm(state: &AppState, account_id: &str, query: &str) -> Option<QueryPlan> {
    let client = state
        .ai
        .resolve(account_id, Capability::Summarize)
        .await
        .ok()?;
    let model = state
        .ai
        .account_config(account_id)
        .await
        .ok()
        .and_then(|c| c.model)
        .unwrap_or_default();
    let req = ChatRequest {
        model,
        system: PLANNER_SYSTEM.to_string(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: query.to_string(),
        }],
        max_tokens: PLANNER_MAX_TOKENS,
        temperature: 0.0,
        stop: Vec::new(),
        purpose: Capability::Summarize,
        request_id: uuid::Uuid::new_v4(),
    };
    let resp = client.chat(req).await.ok()?;
    let parsed: PlanJson = serde_json::from_str(strip_fence(&resp.text)).ok()?;
    Some(plan_from_intent(
        &parsed.intent,
        &parsed.window,
        parsed.unread_only,
    ))
}

/// Build a [`QueryPlan`] from the model's classification, mirroring the fast
/// path's leg assembly so both planners produce identical-shaped plans.
fn plan_from_intent(intent: &str, window: &str, unread_only: bool) -> QueryPlan {
    let win = match window {
        "today" => TimeWindow::Today,
        "week" | "this_week" | "thisweek" => TimeWindow::ThisWeek,
        _ => TimeWindow::AllTime,
    };
    let mut plan = QueryPlan {
        semantic: false,
        memory: false,
        temporal: None,
        aggregates: Vec::new(),
    };
    match intent {
        "overview" => {
            plan.memory = true;
            plan.temporal = Some(TemporalQuery {
                window: win,
                unread_only,
            });
            plan.aggregates = vec![
                AggregateQuery::TotalCount,
                AggregateQuery::UnreadCount,
                AggregateQuery::TopSenders,
            ];
        }
        "recent" => {
            plan.temporal = Some(TemporalQuery {
                window: win,
                unread_only,
            });
        }
        "count" => {
            plan.aggregates = vec![if unread_only {
                AggregateQuery::UnreadCount
            } else {
                AggregateQuery::TotalCount
            }];
        }
        "unread" => {
            plan.aggregates = vec![AggregateQuery::UnreadCount];
            plan.temporal = Some(TemporalQuery {
                window: win,
                unread_only: true,
            });
        }
        "senders" => {
            plan.aggregates = vec![AggregateQuery::TopSenders];
        }
        // "topic" and anything unrecognised → semantic.
        _ => {
            plan.semantic = true;
        }
    }
    plan
}

/// Strip a single wrapping ``` fence (optionally ```json) if present.
fn strip_fence(text: &str) -> &str {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            return rest[..end]
                .split_once('\n')
                .map(|x| x.1)
                .unwrap_or(&rest[..end])
                .trim();
        }
    }
    t
}

/// Leg E — newest-N mails within the planned window, ordered by arrival
/// (`date_received` DESC) and scoped to the active inbox (received mail, not
/// archived or deleted). The citation date shown is the sent date, as elsewhere.
async fn retrieve_temporal(
    state: &AppState,
    account_id: &str,
    query: TemporalQuery,
    limit: usize,
) -> AppResult<Vec<ContextItem>> {
    let since = match query.window {
        TimeWindow::AllTime => None,
        TimeWindow::Today => {
            let now = now_unix();
            Some(now - now.rem_euclid(86_400))
        }
        TimeWindow::ThisWeek => Some(now_unix() - 7 * 86_400),
    };

    let mut sql = String::from(
        "SELECT id, subject, from_email, date_sent, COALESCE(snippet, '') AS snippet \
         FROM mails WHERE account_id = ? AND is_deleted = 0 AND is_archived = 0 AND is_sent = 0",
    );
    if query.unread_only {
        sql.push_str(" AND is_read = 0");
    }
    if since.is_some() {
        sql.push_str(" AND date_received >= ?");
    }
    sql.push_str(" ORDER BY date_received DESC LIMIT ?");

    let mut q = sqlx::query(&sql).bind(account_id);
    if let Some(since) = since {
        q = q.bind(since);
    }
    let rows = q
        .bind(limit as i64)
        .fetch_all(state.storage.db().pool())
        .await
        .map_err(crate::storage::map_sqlx_err)?;

    Ok(rows
        .iter()
        .map(|r| ContextItem {
            kind: ContextItemKind::Temporal,
            mail_id: r.get::<String, _>("id"),
            subject: r.get::<String, _>("subject"),
            from_email: r.get::<String, _>("from_email"),
            date_sent: r.get::<i64, _>("date_sent"),
            content: truncate_chars(&r.get::<String, _>("snippet"), context::CHUNK_SNIPPET_CHARS),
            score: None,
        })
        .collect())
}

/// The account's rolling inbox digest (level-2 reduction) as a single Memory
/// item — the cheapest complete overview, guaranteed into the bundle even when
/// the budget can't fit every per-thread summary (analysis/54 §3.3). `None` until
/// a digest has been built.
async fn retrieve_digest(state: &AppState, account_id: &str) -> AppResult<Option<ContextItem>> {
    Ok(inbox_digest_repo::get(state.storage.db(), account_id)
        .await?
        .map(|d| ContextItem {
            kind: ContextItemKind::Memory,
            mail_id: String::new(),
            subject: String::new(),
            from_email: String::new(),
            date_sent: d.generated_at,
            content: d.digest,
            score: None,
        }))
}

/// Leg F — the precomputed thread-summary memory (P-4). Reads the newest stored
/// one-line summaries for the account (no AI call on this path), so "summarise
/// everything" packs one line per thread instead of every raw mail — the
/// map-reduce shortcut from analysis/54 §3.3/§3.5. Each item is synthesised over
/// a whole thread, so it carries no single `mail_id`.
async fn retrieve_memory(
    state: &AppState,
    account_id: &str,
    limit: usize,
) -> AppResult<Vec<ContextItem>> {
    let summaries =
        thread_summary_repo::list_recent(state.storage.db(), account_id, limit as i64).await?;
    Ok(summaries
        .into_iter()
        .map(|s| ContextItem {
            kind: ContextItemKind::Memory,
            mail_id: String::new(),
            subject: String::new(),
            from_email: String::new(),
            date_sent: s.latest_date,
            content: s.summary,
            score: None,
        })
        .collect())
}

/// One thread's precomputed summary as a Memory [`ContextItem`], if present — the
/// anchored counterpart to [`retrieve_digest`] (analysis/55 §4). Reused by the
/// reply path so a draft sees this conversation's long-term arc. Synthesised over
/// a whole thread, so it carries no single `mail_id`.
async fn retrieve_thread_summary(
    state: &AppState,
    thread_id: &str,
) -> AppResult<Option<ContextItem>> {
    Ok(thread_summary_repo::get(state.storage.db(), thread_id)
        .await?
        .map(|s| ContextItem {
            kind: ContextItemKind::Memory,
            mail_id: String::new(),
            subject: String::new(),
            from_email: String::new(),
            date_sent: s.latest_date,
            content: s.summary,
            score: None,
        }))
}

/// Anchored Memory leg (analysis/55 §4, Gap A). The reply / analysis path used to
/// see only target + thread + semantic context, so a draft never knew this
/// conversation's long-term arc or the wider mailbox state. Top the assembled
/// anchored context up with the precomputed memory layer, within whatever token
/// budget the anchored legs left:
///
/// 1. this thread's own summary — the most relevant arc for a reply;
/// 2. the rolling inbox digest — broad situational awareness.
///
/// Both are read-only (no AI call on this path) and short. Memory items carry no
/// source `mail_id`, so `knowledge_refs` is untouched. Missing summaries/digest
/// are normal early on and simply add nothing.
async fn append_anchored_memory(
    state: &AppState,
    p: &RoleContextParams,
    ctx: &mut AssembledContext,
) -> AppResult<()> {
    let thread_summary = match &p.thread_id {
        Some(thread_id) => retrieve_thread_summary(state, thread_id).await?,
        None => None,
    };
    let digest = retrieve_digest(state, &p.account_id).await?;

    let mut used = ctx.total_tokens_used;
    // Thread summary leads (most relevant), then the mailbox digest — the same
    // priority-packing idiom as the question path: stop once the budget is hit.
    for item in thread_summary.into_iter().chain(digest) {
        let cost = context::estimate_tokens(&item.content);
        if used + cost > p.token_budget {
            break;
        }
        used += cost;
        ctx.report.memory_hits += 1;
        ctx.items.push(item);
    }
    ctx.total_tokens_used = used;

    tracing::info!(
        event = "mce_mail_memory_appended",
        account_id = %p.account_id,
        capability = p.capability.as_str(),
        memory_hits = ctx.report.memory_hits,
        tokens = ctx.total_tokens_used,
        budget = p.token_budget,
        "appended anchored memory to the reply/analysis context"
    );
    Ok(())
}

/// Leg D — the named counterparty's profile from `contacts` (analysis/54 §3.2).
/// An explicit email matches exactly; a name keyword is LIKE-matched against
/// email / display name / organisation, most-interacted first. Each row becomes a
/// compact Sender [`ContextItem`]. Contacts are not account-scoped (a counterparty
/// is the same across mailboxes) and carry no source `mail_id`.
async fn retrieve_sender(
    state: &AppState,
    sender: &SenderQuery,
    limit: usize,
) -> AppResult<Vec<ContextItem>> {
    let db = state.storage.db().pool();
    const COLS: &str = "email, display_name, organisation, interaction_count, reply_count, \
                        avg_reply_hours, trust_score, style_notes";

    let rows = if let Some(email) = &sender.email {
        sqlx::query(&format!(
            "SELECT {COLS} FROM contacts WHERE email = ? LIMIT ?"
        ))
        .bind(email.to_lowercase())
        .bind(limit as i64)
        .fetch_all(db)
        .await
    } else if let Some(name) = &sender.name {
        let like = format!("%{}%", name.to_lowercase());
        sqlx::query(&format!(
            "SELECT {COLS} FROM contacts \
             WHERE lower(email) LIKE ? OR lower(COALESCE(display_name, '')) LIKE ? \
                OR lower(COALESCE(organisation, '')) LIKE ? \
             ORDER BY interaction_count DESC, email ASC LIMIT ?"
        ))
        .bind(&like)
        .bind(&like)
        .bind(&like)
        .bind(limit as i64)
        .fetch_all(db)
        .await
    } else {
        return Ok(Vec::new());
    }
    .map_err(crate::storage::map_sqlx_err)?;

    Ok(rows
        .iter()
        .map(|r| {
            let email: String = r.get("email");
            ContextItem {
                kind: ContextItemKind::Sender,
                mail_id: String::new(),
                subject: String::new(),
                from_email: email.clone(),
                date_sent: 0,
                content: format_sender_profile(
                    &email,
                    r.get::<Option<String>, _>("display_name").as_deref(),
                    r.get::<Option<String>, _>("organisation").as_deref(),
                    r.get::<i64, _>("interaction_count"),
                    r.get::<i64, _>("reply_count"),
                    r.get::<Option<f64>, _>("avg_reply_hours"),
                    r.get::<f64, _>("trust_score"),
                    r.get::<Option<String>, _>("style_notes").as_deref(),
                ),
                score: None,
            }
        })
        .collect())
}

/// A compact one-line English profile for a contact row (leg D rendering).
#[allow(clippy::too_many_arguments)]
fn format_sender_profile(
    email: &str,
    display_name: Option<&str>,
    organisation: Option<&str>,
    interactions: i64,
    replies: i64,
    avg_reply_hours: Option<f64>,
    trust_score: f64,
    style_notes: Option<&str>,
) -> String {
    let mut s = format!("Contact {email}");
    match (
        display_name.map(str::trim).filter(|v| !v.is_empty()),
        organisation.map(str::trim).filter(|v| !v.is_empty()),
    ) {
        (Some(n), Some(o)) => s.push_str(&format!(" ({n}, {o})")),
        (Some(n), None) => s.push_str(&format!(" ({n})")),
        (None, Some(o)) => s.push_str(&format!(" ({o})")),
        (None, None) => {}
    }
    s.push_str(&format!(
        ": {interactions} exchanges, you replied {replies}"
    ));
    if let Some(h) = avg_reply_hours {
        s.push_str(&format!(", avg reply {h:.1}h"));
    }
    s.push_str(&format!(", trust {trust_score:.2}."));
    if let Some(note) = style_notes.and_then(humanize_style_notes) {
        let note = truncate_chars(note.trim(), 200);
        if !note.is_empty() {
            s.push_str(&format!(" Notes: {note}"));
        }
    }
    s
}

/// Pull a human-readable note out of the `contacts.style_notes` JSON: the string
/// itself when it's a bare string, or the first non-empty value of a known key
/// (`relationship` / `summary` / `notes` / `style` / `tone`) when it's an object.
fn humanize_style_notes(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    match value {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Object(map) => ["relationship", "summary", "notes", "style", "tone"]
            .iter()
            .find_map(|key| match map.get(*key) {
                Some(serde_json::Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
                _ => None,
            }),
        _ => None,
    }
}

/// Leg C — one computed structured fact over the active inbox (received mail,
/// not archived or deleted). Returns `None` only when there is nothing to report
/// (e.g., no senders yet); a zero count is still a useful fact and returns
/// `Some`.
async fn compute_aggregate(
    state: &AppState,
    account_id: &str,
    query: AggregateQuery,
) -> AppResult<Option<ContextItem>> {
    const INBOX_SCOPE: &str = "is_deleted = 0 AND is_archived = 0 AND is_sent = 0";
    let db = state.storage.db().pool();

    let content = match query {
        AggregateQuery::TotalCount => {
            let (n,): (i64,) = sqlx::query_as(&format!(
                "SELECT COUNT(*) FROM mails WHERE account_id = ? AND {INBOX_SCOPE}"
            ))
            .bind(account_id)
            .fetch_one(db)
            .await
            .map_err(crate::storage::map_sqlx_err)?;
            format!("Emails in the inbox: {n}.")
        }
        AggregateQuery::UnreadCount => {
            let (n,): (i64,) = sqlx::query_as(&format!(
                "SELECT COUNT(*) FROM mails WHERE account_id = ? AND {INBOX_SCOPE} AND is_read = 0"
            ))
            .bind(account_id)
            .fetch_one(db)
            .await
            .map_err(crate::storage::map_sqlx_err)?;
            format!("Unread emails in the inbox: {n}.")
        }
        AggregateQuery::TopSenders => {
            let rows = sqlx::query(&format!(
                "SELECT from_email, COUNT(*) AS c FROM mails \
                 WHERE account_id = ? AND {INBOX_SCOPE} \
                 GROUP BY from_email ORDER BY c DESC, from_email ASC LIMIT 5"
            ))
            .bind(account_id)
            .fetch_all(db)
            .await
            .map_err(crate::storage::map_sqlx_err)?;
            if rows.is_empty() {
                return Ok(None);
            }
            let list = rows
                .iter()
                .map(|r| {
                    format!(
                        "{} ({})",
                        r.get::<String, _>("from_email"),
                        r.get::<i64, _>("c")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("Most frequent senders in the inbox: {list}.")
        }
    };

    Ok(Some(ContextItem {
        kind: ContextItemKind::Aggregate,
        mail_id: String::new(),
        subject: String::new(),
        from_email: String::new(),
        date_sent: 0,
        content,
        score: None,
    }))
}

/// Index coverage for one account: `(indexed, total)` over stored, non-deleted
/// mail. `indexed` counts `embedding_status = 'indexed'` (the same signal the
/// GTE stats page uses). One cheap conditional-count query; surfaced so the chat
/// UI can say "searched N of M" while the index is still building.
async fn index_coverage(state: &AppState, account_id: &str) -> AppResult<(usize, usize)> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS total, \
                COALESCE(SUM(CASE WHEN embedding_status = 'indexed' THEN 1 ELSE 0 END), 0) AS indexed \
         FROM mails WHERE account_id = ? AND is_deleted = 0",
    )
    .bind(account_id)
    .fetch_one(state.storage.db().pool())
    .await
    .map_err(crate::storage::map_sqlx_err)?;
    let total: i64 = row.get("total");
    let indexed: i64 = row.get("indexed");
    Ok((indexed.max(0) as usize, total.max(0) as usize))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::ProviderError;
    use crate::ai::types::{AiProvider, ChatResponse, FinishReason, TokenUsage};
    use crate::util::now_unix;
    use crate::vector::VectorRow;

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

    async fn seed_thread_row(state: &AppState, id: &str, acc: &str, mail_count: i64, latest: i64) {
        sqlx::query(
            "INSERT INTO threads (id, account_id, subject, participants, mail_count, unread_count, \
                 latest_date, created_at, updated_at) \
             VALUES (?, ?, 'Subject', '[]', ?, 0, ?, 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(mail_count)
        .bind(latest)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    async fn seed_summary(state: &AppState, thread: &str, acc: &str, summary: &str, latest: i64) {
        thread_summary_repo::upsert(
            state.storage.db(),
            &thread_summary_repo::ThreadSummaryInput {
                thread_id: thread.into(),
                account_id: acc.into(),
                summary: summary.into(),
                key_entities: Vec::new(),
                mail_count: 1,
                latest_date: latest,
                model: None,
            },
        )
        .await
        .unwrap();
    }

    async fn seed_digest(state: &AppState, acc: &str, digest: &str) {
        inbox_digest_repo::upsert(
            state.storage.db(),
            &inbox_digest_repo::InboxDigestInput {
                account_id: acc.into(),
                digest: digest.into(),
                thread_count: 5,
                unread_count: 2,
                model: None,
            },
        )
        .await
        .unwrap();
    }

    async fn seed_ai_settings(state: &AppState, acc: &str) {
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, updated_at) \
             VALUES (?, 1, 'openai', 'gpt-4o', 0)",
        )
        .bind(acc)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    fn ok_chat(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            },
            model_echo: "gpt-4o".into(),
            latency_ms: 30,
        })
    }

    async fn seed_mail(
        state: &AppState,
        id: &str,
        acc: &str,
        from_email: &str,
        body: &str,
        date_sent: i64,
    ) {
        sqlx::query(
            "INSERT INTO mails (id, account_id, thread_id, message_id, subject, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, embedding_status, \
                 created_at, updated_at) \
             VALUES (?, ?, NULL, ?, ?, ?, '[]', ?, ?, ?, ?, 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(acc)
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

    const RENEWAL_BODY: &str =
        "the quarterly licensing contract renewal terms and the indemnity clause review";
    const QUERY: &str = "licensing contract renewal indemnity clause";

    #[tokio::test]
    async fn question_grounds_on_semantic_hits() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", Some("Coordinate vendor contracts.")).await;
        seed_mail(&state, "m1", "a", "peer@x.com", RENEWAL_BODY, now_unix()).await;
        index_mail(&state, "m1", "a", &[RENEWAL_BODY]).await;
        let related = "prior licensing contract renewal discussion and indemnity terms";
        seed_mail(&state, "m2", "a", "peer@x.com", related, now_unix() - 100).await;
        index_mail(&state, "m2", "a", &[related]).await;

        let qp = QuestionParams::new(QUERY, "a", 100_000, Capability::Summarize);
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        assert!(!ctx.items.is_empty(), "related mail should be retrieved");
        assert!(ctx
            .items
            .iter()
            .all(|i| i.kind == ContextItemKind::Semantic));
        assert_eq!(ctx.knowledge_refs.len(), ctx.items.len());
        // Score-descending invariant.
        for pair in ctx.items.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
        assert!(!ctx.report.anchored);
        assert!(ctx.report.semantic_available);
        assert_eq!(ctx.report.semantic_hits, ctx.items.len());
        assert!(ctx.contact_history.is_none());
        assert!(ctx.total_tokens_used <= qp.token_budget);
        assert_eq!(ctx.safety_preamble, SAFETY_PREAMBLE);
        assert!(ctx.role_preamble.contains("a@x.com"));
    }

    #[tokio::test]
    async fn empty_mailbox_reports_available_but_no_hits() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;

        let qp = QuestionParams::new("anything for me?", "a", 100_000, Capability::Summarize);
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        assert!(ctx.items.is_empty());
        assert!(ctx.knowledge_refs.is_empty());
        // The index ran fine; there was simply nothing to match — distinct from
        // an index that could not run at all.
        assert!(ctx.report.semantic_available);
        assert!(!ctx.report.anchored);
        assert_eq!(ctx.report.semantic_hits, 0);
    }

    #[tokio::test]
    async fn question_packs_within_budget() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_mail(&state, "m1", "a", "peer@x.com", RENEWAL_BODY, now_unix()).await;
        index_mail(&state, "m1", "a", &[RENEWAL_BODY]).await;
        let related = "prior licensing contract renewal discussion and indemnity terms";
        seed_mail(&state, "m2", "a", "peer@x.com", related, now_unix() - 100).await;
        index_mail(&state, "m2", "a", &[related]).await;

        let engine = MailboxContextEngine::new(&state);
        let full = engine
            .assemble_for_question(&QuestionParams::new(
                QUERY,
                "a",
                100_000,
                Capability::Summarize,
            ))
            .await
            .unwrap();
        assert!(full.items.len() >= 2, "both related mails retrieved");

        // One token under the full cost drops the lowest-scoring item.
        let tight = QuestionParams::new(
            QUERY,
            "a",
            full.total_tokens_used - 1,
            Capability::Summarize,
        );
        let trimmed = engine.assemble_for_question(&tight).await.unwrap();
        assert!(trimmed.items.len() < full.items.len());
        assert_eq!(trimmed.knowledge_refs.len(), trimmed.items.len());
        assert!(trimmed.total_tokens_used <= tight.token_budget);
    }

    #[tokio::test]
    async fn unknown_account_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        let qp = QuestionParams::new("hi", "ghost", 1_000, Capability::Summarize);
        let err = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound));
    }

    #[tokio::test]
    async fn anchored_adapter_maps_role_context() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(
            &state,
            "a",
            "legal",
            Some("Review inbound contracts for risk."),
        )
        .await;
        seed_mail(
            &state,
            "trigger",
            "a",
            "peer@x.com",
            RENEWAL_BODY,
            now_unix(),
        )
        .await;
        index_mail(&state, "trigger", "a", &[RENEWAL_BODY]).await;
        let related = "prior licensing contract renewal discussion and indemnity terms";
        seed_mail(&state, "k1", "a", "peer@x.com", related, now_unix() - 100).await;
        index_mail(&state, "k1", "a", &[related]).await;
        seed_contact(&state, "peer@x.com", 12, 7).await;

        let p = RoleContextParams::new("trigger", "a", 100_000, Capability::RiskReason);
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_mail(&p)
            .await
            .unwrap();

        assert!(ctx.report.anchored);
        // Target mail leads, untouched.
        let first = ctx.items.first().expect("target item present");
        assert_eq!(first.kind, ContextItemKind::Target);
        assert_eq!(first.mail_id, "trigger");
        assert_eq!(first.content, RENEWAL_BODY);
        // Semantic items follow; refs == one per distinct semantic mail.
        let semantic = ctx.items_of(ContextItemKind::Semantic).count();
        assert!(semantic >= 1, "prior mail grounds the analysis");
        assert_eq!(ctx.knowledge_refs.len(), semantic);
        assert!(ctx.knowledge_refs.iter().all(|id| id != "trigger"));
        // Contact history wired through for the analysis capability.
        let history = ctx.contact_history.expect("contact row exists");
        assert_eq!(history.data.interaction_count, 12);
        assert!(ctx.total_tokens_used <= p.token_budget);
    }

    /// Helper: seed this thread's summary + the account's rolling inbox digest.
    async fn seed_memory(state: &AppState, account: &str, thread: &str) {
        seed_thread_row(state, thread, account, 2, now_unix()).await;
        thread_summary_repo::upsert(
            state.storage.db(),
            &thread_summary_repo::ThreadSummaryInput {
                thread_id: thread.into(),
                account_id: account.into(),
                summary: "Acme renewal: pricing agreed, indemnity clause still open.".into(),
                key_entities: vec!["Acme".into()],
                mail_count: 2,
                latest_date: now_unix(),
                model: Some("gpt-4o".into()),
            },
        )
        .await
        .unwrap();
        inbox_digest_repo::upsert(
            state.storage.db(),
            &inbox_digest_repo::InboxDigestInput {
                account_id: account.into(),
                digest: "Three vendor threads active; one invoice overdue.".into(),
                thread_count: 3,
                unread_count: 1,
                model: Some("gpt-4o".into()),
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn anchored_path_appends_thread_summary_and_digest() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(
            &state,
            "a",
            "legal",
            Some("Review inbound contracts for risk."),
        )
        .await;
        seed_mail(
            &state,
            "trigger",
            "a",
            "peer@x.com",
            RENEWAL_BODY,
            now_unix(),
        )
        .await;
        seed_memory(&state, "a", "t1").await;

        let mut p = RoleContextParams::new("trigger", "a", 100_000, Capability::RiskReason);
        p.thread_id = Some("t1".into());
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_mail(&p)
            .await
            .unwrap();

        // The anchored path now carries the memory leg (analysis/55 §4): this
        // thread's summary plus the rolling inbox digest.
        let memory: Vec<&ContextItem> = ctx.items_of(ContextItemKind::Memory).collect();
        assert_eq!(memory.len(), 2, "thread summary + inbox digest");
        assert_eq!(ctx.report.memory_hits, 2);
        assert!(memory
            .iter()
            .any(|m| m.content.contains("indemnity clause still open")));
        assert!(memory
            .iter()
            .any(|m| m.content.contains("one invoice overdue")));
        // Memory items are summaries, not source mails: refs stay clean.
        assert!(memory.iter().all(|m| m.mail_id.is_empty()));
        assert!(ctx.knowledge_refs.iter().all(|id| id != "trigger"));
        assert!(ctx.total_tokens_used <= p.token_budget);
    }

    #[tokio::test]
    async fn anchored_memory_respects_token_budget() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(
            &state,
            "a",
            "legal",
            Some("Review inbound contracts for risk."),
        )
        .await;
        seed_mail(
            &state,
            "trigger",
            "a",
            "peer@x.com",
            RENEWAL_BODY,
            now_unix(),
        )
        .await;
        seed_memory(&state, "a", "t1").await;

        // Roomy budget fits both memory items.
        let mut roomy = RoleContextParams::new("trigger", "a", 100_000, Capability::RiskReason);
        roomy.thread_id = Some("t1".into());
        let full = MailboxContextEngine::new(&state)
            .assemble_for_mail(&roomy)
            .await
            .unwrap();
        assert_eq!(full.report.memory_hits, 2);

        // One token under the full total: at least the last memory item no longer
        // fits, and the result still respects the budget.
        let mut tight = RoleContextParams::new("trigger", "a", 100_000, Capability::RiskReason);
        tight.thread_id = Some("t1".into());
        tight.token_budget = full.total_tokens_used - 1;
        let trimmed = MailboxContextEngine::new(&state)
            .assemble_for_mail(&tight)
            .await
            .unwrap();
        assert!(trimmed.report.memory_hits < full.report.memory_hits);
        assert!(trimmed.total_tokens_used <= tight.token_budget);
    }

    // ── Sender leg D (analysis/54 §3.2) ─────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    async fn seed_rich_contact(
        state: &AppState,
        email: &str,
        display_name: Option<&str>,
        organisation: Option<&str>,
        interactions: i64,
        replies: i64,
        trust: f64,
        style_notes: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO contacts (id, email, display_name, organisation, first_seen_at, \
                 last_seen_at, interaction_count, reply_count, avg_reply_hours, trust_score, \
                 style_notes, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 0, 0, ?, ?, 4.0, ?, ?, 0, 0)",
        )
        .bind(crate::util::new_uuid())
        .bind(email)
        .bind(display_name)
        .bind(organisation)
        .bind(interactions)
        .bind(replies)
        .bind(trust)
        .bind(style_notes)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    #[test]
    fn detect_sender_finds_email_then_name_then_none() {
        let by_email = detect_sender("what's my history with Alice@Acme.COM?").unwrap();
        assert_eq!(by_email.email.as_deref(), Some("alice@acme.com"));
        assert!(by_email.name.is_none());

        let by_name = detect_sender("what is my relationship with Globex.").unwrap();
        assert!(by_name.email.is_none());
        assert_eq!(by_name.name.as_deref(), Some("globex"));

        assert!(detect_sender("how many unread emails do I have?").is_none());
    }

    #[tokio::test]
    async fn question_surfaces_sender_profile_by_email() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_rich_contact(
            &state,
            "alice@acme.com",
            Some("Alice Smith"),
            Some("Acme"),
            42,
            30,
            0.8,
            Some(r#"{"relationship":"Key renewals contact; prefers concise updates."}"#),
        )
        .await;

        let qp = QuestionParams::new(
            "what's my history with alice@acme.com?",
            "a",
            100_000,
            Capability::Summarize,
        );
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        assert_eq!(ctx.report.sender_hits, 1);
        let sender: Vec<&ContextItem> = ctx.items_of(ContextItemKind::Sender).collect();
        assert_eq!(sender.len(), 1);
        let content = &sender[0].content;
        assert!(content.contains("alice@acme.com"));
        assert!(content.contains("42 exchanges"));
        assert!(content.contains("trust 0.80"));
        assert!(
            content.contains("Key renewals contact"),
            "style note surfaced"
        );
        // Profiles aren't source mails → never cited.
        assert!(sender[0].mail_id.is_empty());
        assert!(ctx.total_tokens_used <= qp.token_budget);
    }

    #[tokio::test]
    async fn question_surfaces_sender_profile_by_name() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_rich_contact(
            &state,
            "billing@globex.com",
            Some("Bob"),
            Some("Globex"),
            5,
            2,
            0.5,
            None,
        )
        .await;

        let qp = QuestionParams::new("tell me about Globex.", "a", 100_000, Capability::Summarize);
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        assert_eq!(ctx.report.sender_hits, 1);
        assert!(ctx
            .items_of(ContextItemKind::Sender)
            .next()
            .unwrap()
            .content
            .contains("Globex"));
    }

    // ── Fast-path planner (P-2 §3.1) ────────────────────────────────────────

    #[test]
    fn planner_routes_overview_to_memory_aggregate_and_temporal() {
        let plan = plan_question("Can you summarize my inbox?");
        assert!(plan.memory, "overview prefers precomputed thread summaries");
        assert!(
            plan.temporal.is_some(),
            "temporal stays as the day-one fallback"
        );
        assert!(plan.aggregates.contains(&AggregateQuery::TotalCount));
        assert!(plan.aggregates.contains(&AggregateQuery::UnreadCount));
        assert!(plan.aggregates.contains(&AggregateQuery::TopSenders));
        assert!(!plan.semantic);
    }

    #[test]
    fn planner_routes_unread_count() {
        let plan = plan_question("how many unread emails do I have?");
        assert_eq!(plan.aggregates, vec![AggregateQuery::UnreadCount]);
        assert!(plan.temporal.is_none());
        assert!(!plan.semantic);
    }

    #[test]
    fn planner_routes_today_to_temporal_window() {
        let plan = plan_question("what came in today?");
        let t = plan.temporal.expect("temporal leg planned");
        assert_eq!(t.window, TimeWindow::Today);
        assert!(plan.aggregates.is_empty());
        assert!(!plan.semantic);
    }

    #[test]
    fn planner_routes_top_senders() {
        let plan = plan_question("who emails me the most?");
        assert_eq!(plan.aggregates, vec![AggregateQuery::TopSenders]);
        assert!(!plan.semantic);
    }

    #[test]
    fn planner_unread_this_week_combines_window_and_scope() {
        let plan = plan_question("show me unread mail from this week");
        let t = plan.temporal.expect("temporal leg planned");
        assert_eq!(t.window, TimeWindow::ThisWeek);
        assert!(t.unread_only);
        assert_eq!(plan.aggregates, vec![AggregateQuery::UnreadCount]);
        assert!(!plan.semantic);
    }

    #[test]
    fn planner_falls_back_to_semantic_for_topics() {
        let plan = plan_question("emails about the licensing contract renewal");
        assert!(plan.semantic);
        assert!(plan.temporal.is_none());
        assert!(plan.aggregates.is_empty());
    }

    // ── Slow-path model planner (P-5) ───────────────────────────────────────

    #[test]
    fn plan_from_intent_maps_intents() {
        let overview = plan_from_intent("overview", "week", false);
        assert!(overview.memory);
        assert_eq!(overview.temporal.unwrap().window, TimeWindow::ThisWeek);
        assert!(overview.aggregates.contains(&AggregateQuery::TopSenders));
        assert!(!overview.semantic);

        let unread = plan_from_intent("unread", "all", false);
        assert_eq!(unread.aggregates, vec![AggregateQuery::UnreadCount]);
        assert!(unread.temporal.unwrap().unread_only);

        let topic = plan_from_intent("topic", "all", false);
        assert!(topic.semantic);
        assert!(topic.temporal.is_none());
    }

    #[tokio::test]
    async fn model_planner_reroutes_non_keyword_overview() {
        let (state, _rx) = AppState::test_state().await;
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        // The model classifies a keyword-less question as an inbox overview.
        mock.push_chat(ok_chat(
            "{\"intent\":\"overview\",\"window\":\"all\",\"unread_only\":false}",
        ));
        state.ai.register(mock.clone());
        seed_account(&state, "a", "work", None).await;
        seed_ai_settings(&state, "a").await;
        seed_thread_row(&state, "t1", "a", 1, 200).await;
        seed_summary(&state, "t1", "a", "Renewal A pending.", 200).await;

        // A phrasing with no English routing keywords → the fast path gives up
        // (semantic), so the slow planner runs and reroutes to the overview.
        let mut qp = QuestionParams::new(
            "给我讲讲我的邮箱整体情况",
            "a",
            100_000,
            Capability::Summarize,
        );
        qp.allow_model_planner = true;
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        assert_eq!(
            ctx.report.memory_hits, 1,
            "model rerouted to the memory leg"
        );
        assert_eq!(ctx.report.semantic_hits, 0);
    }

    #[tokio::test]
    async fn model_planner_off_by_default_keeps_fast_path() {
        let (state, _rx) = AppState::test_state().await;
        // A mock that would error if called — proves no planner call happens.
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        mock.set_default_chat_error(ProviderError::Unreachable("must not be called".into()));
        state.ai.register(mock.clone());
        seed_account(&state, "a", "work", None).await;
        seed_ai_settings(&state, "a").await;

        // Keyword-less question, planner NOT allowed → stays on the fast path
        // (semantic), no model call, no panic.
        let qp = QuestionParams::new(
            "给我讲讲我的邮箱整体情况",
            "a",
            100_000,
            Capability::Summarize,
        );
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();
        assert_eq!(ctx.report.memory_hits, 0);
        assert_eq!(mock.chat_call_count(), 0, "no planner call without opt-in");
    }

    // ── Aggregate leg C / Temporal leg E (P-2 §3.2) ─────────────────────────

    #[tokio::test]
    async fn question_aggregate_reports_unread_count() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_mail(&state, "m1", "a", "x@p.com", "hello one", now_unix()).await;
        seed_mail(&state, "m2", "a", "y@p.com", "hello two", now_unix() - 10).await;
        seed_mail(&state, "m3", "a", "z@p.com", "hello three", now_unix() - 20).await;
        // Mark one read → 2 unread remain.
        sqlx::query("UPDATE mails SET is_read = 1 WHERE id = 'm3'")
            .execute(state.storage.db().pool())
            .await
            .unwrap();

        let qp = QuestionParams::new(
            "how many unread emails do I have?",
            "a",
            100_000,
            Capability::Summarize,
        );
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        assert_eq!(ctx.report.aggregate_facts, 1);
        assert_eq!(ctx.report.semantic_hits, 0);
        assert_eq!(ctx.report.temporal_hits, 0);
        assert!(ctx.items.iter().any(|i| {
            i.kind == ContextItemKind::Aggregate
                && i.content.contains("Unread emails in the inbox: 2")
        }));
        // Computed facts carry no source mail → no citable refs.
        assert!(ctx.knowledge_refs.is_empty());
        assert!(ctx.total_tokens_used <= qp.token_budget);
    }

    #[tokio::test]
    async fn question_temporal_returns_newest_first() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_mail(&state, "old", "a", "x@p.com", "old news", now_unix() - 1000).await;
        seed_mail(&state, "mid", "a", "y@p.com", "mid news", now_unix() - 500).await;
        seed_mail(&state, "new", "a", "z@p.com", "fresh news", now_unix() - 10).await;

        let qp = QuestionParams::new(
            "what are my most recent emails?",
            "a",
            100_000,
            Capability::Summarize,
        );
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        let temporal: Vec<_> = ctx.items_of(ContextItemKind::Temporal).collect();
        assert_eq!(temporal.len(), 3);
        assert_eq!(temporal[0].mail_id, "new", "newest arrives first");
        assert_eq!(temporal[2].mail_id, "old");
        assert_eq!(ctx.report.temporal_hits, 3);
        assert_eq!(ctx.report.semantic_hits, 0);
        // Temporal mails are real emails → cited.
        assert_eq!(ctx.knowledge_refs.len(), 3);
    }

    #[tokio::test]
    async fn question_summarize_runs_aggregate_and_temporal() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_mail(&state, "m1", "a", "x@p.com", "first", now_unix() - 5).await;
        seed_mail(&state, "m2", "a", "y@p.com", "second", now_unix() - 50).await;

        let qp = QuestionParams::new(
            "summarize my inbox please",
            "a",
            100_000,
            Capability::Summarize,
        );
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        // Overview → total + unread + top senders, plus the newest mails.
        assert_eq!(ctx.report.aggregate_facts, 3);
        assert_eq!(ctx.report.temporal_hits, 2);
        assert_eq!(ctx.report.semantic_hits, 0);
        // No summaries built yet → the memory leg is empty and temporal carries
        // the overview (day-one behaviour).
        assert_eq!(ctx.report.memory_hits, 0);
        // Only the real mails (temporal) are cited; the three facts are not.
        assert_eq!(ctx.knowledge_refs.len(), ctx.report.temporal_hits);
        assert!(ctx.total_tokens_used <= qp.token_budget);
    }

    #[tokio::test]
    async fn question_summarize_uses_thread_memory_when_present() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_thread_row(&state, "t1", "a", 1, 200).await;
        seed_thread_row(&state, "t2", "a", 1, 100).await;
        seed_summary(&state, "t1", "a", "Acme renewal pending sign-off.", 200).await;
        seed_summary(&state, "t2", "a", "Invoice settled.", 100).await;

        let qp = QuestionParams::new("summarize my inbox", "a", 100_000, Capability::Summarize);
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        // Overview now reads precomputed summaries instead of dumping raw mail.
        assert_eq!(ctx.report.memory_hits, 2);
        assert_eq!(
            ctx.report.temporal_hits, 0,
            "memory replaces the temporal fallback"
        );
        let mem: Vec<_> = ctx.items_of(ContextItemKind::Memory).collect();
        assert_eq!(mem.len(), 2);
        assert_eq!(
            mem[0].content, "Acme renewal pending sign-off.",
            "newest thread first"
        );
        // Summaries are synthesised over threads, not single mails → not cited.
        assert!(ctx.knowledge_refs.is_empty());
    }

    #[tokio::test]
    async fn question_summarize_leads_with_inbox_digest() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_thread_row(&state, "t1", "a", 1, 200).await;
        seed_summary(&state, "t1", "a", "Renewal A pending.", 200).await;
        seed_digest(
            &state,
            "a",
            "Two renewals await sign-off; the rest is routine.",
        )
        .await;

        let qp = QuestionParams::new("summarize my inbox", "a", 100_000, Capability::Summarize);
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        let mem: Vec<_> = ctx.items_of(ContextItemKind::Memory).collect();
        // The level-2 digest leads, then the per-thread summary fills in detail.
        assert_eq!(
            mem[0].content,
            "Two renewals await sign-off; the rest is routine."
        );
        assert!(mem.iter().any(|i| i.content == "Renewal A pending."));
        assert_eq!(ctx.report.memory_hits, 2);
        assert_eq!(
            ctx.report.temporal_hits, 0,
            "digest+summaries replace raw recent mail"
        );
    }

    #[tokio::test]
    async fn question_report_includes_index_coverage() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "a", "work", None).await;
        seed_mail(&state, "m1", "a", "x@p.com", "one", now_unix()).await;
        seed_mail(&state, "m2", "a", "x@p.com", "two", now_unix() - 10).await;
        seed_mail(&state, "m3", "a", "x@p.com", "three", now_unix() - 20).await;
        // `seed_mail` defaults to embedding_status='indexed'; leave one pending.
        sqlx::query("UPDATE mails SET embedding_status = 'pending' WHERE id = 'm3'")
            .execute(state.storage.db().pool())
            .await
            .unwrap();

        let qp = QuestionParams::new(
            "how many emails total?",
            "a",
            100_000,
            Capability::Summarize,
        );
        let ctx = MailboxContextEngine::new(&state)
            .assemble_for_question(&qp)
            .await
            .unwrap();

        assert_eq!(
            ctx.report.total_mails, 3,
            "all stored mail counts toward total"
        );
        assert_eq!(
            ctx.report.indexed_mails, 2,
            "only embedded mail counts as indexed"
        );
    }
}
