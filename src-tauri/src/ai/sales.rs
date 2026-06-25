//! Sales-role analysis pipeline — D2 Business Negotiation Assistant (T072, F_D2).
//!
//! [`SalesAnalysisPipeline::run`] turns one mail id into a structured
//! [`SalesAnalysisResult`]:
//!
//! 1. **24-hour cache** (`force_new = false`): the most recent D2
//!    `ai_decisions` row within 24 h (`impact = 'context'`,
//!    `result_description LIKE 'D2 sales%'`) is returned as-is — no provider
//!    call, no `daily_query_limit` spend (T072 §3 step 1).
//! 2. **Context** via T074 [`assemble_role_context`] with
//!    `Capability::RiskReason` (role preamble, safety preamble, thread
//!    snippets, GTE chunks, `knowledge_refs`, and the counterparty's
//!    `contacts` history — F_D2 §4.2).
//! 3. **Stance intensity** from `account_ai_settings.style_profile` JSON
//!    (`sales_stance`: `gentle` / `balanced` / `assertive`, default
//!    `balanced`), injected into the system prompt (F_D2 §4.3, T072 §3 step 3).
//! 4. **Marketing-mail guard** (F_D2 §6): when `mails.spam_score >= 0.7` or the
//!    sender address looks like a bulk sender, the prompt instructs the model
//!    to mark the counterparty as `neutral` and note the mass-mailing — the
//!    analysis is never refused outright.
//! 5. **Provider call** through `AiRegistry::resolve(account, RiskReason)` —
//!    non-streaming `chat()` at `temperature = 0.0` (dev/06 §2.1).
//! 6. **Strict JSON validation** against the D2 schema (F_D2 §4.4): unknown
//!    `stance`/`tone`/`priority`/`timeline` values, missing fields, and an
//!    empty `next_actions` are rejected; one re-prompt retry, then `INTERNAL`.
//! 7. **Persistence**: one append-only `ai_decisions` audit row
//!    (`decision_type = 'draft_created'` — see the T072 §11 open question on
//!    a future `role_analysis_d2` enum value — `impact = 'context'`,
//!    `knowledge_refs`, token/latency accounting, and the full result JSON in
//!    `action_description` so the 24-hour cache can replay it). D2 **never
//!    writes `risk_events`** — business judgement is context assistance, not a
//!    safety risk (T072 §3 step 9).
//!
//! **Privacy red-line (dev/09 §5):** this module logs identifiers, counts, and
//! enum tags only — never `body_text`, `evidence` snippets, or any other mail
//! or model content. The `result_description` summary is statistics-only.

use std::collections::HashMap;

use sqlx::Row;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::{
    AnalyzeSalesContextParams, ConcessionAdvice, ContactHistorySummary, CounterpartyProfile,
    CounterpartyStance, CounterpartyTone, NeedItem, NeedPriority, NextAction, NextActionTimeline,
    SalesAnalysisResult,
};
use crate::util::{new_uuid, now_unix, truncate_chars};

use super::context::RoleContextParams;
use super::mce::{AssembledContext, ContextItemKind, MailboxContextEngine};
use super::provider::AiProviderClient;
use super::types::{Capability, ChatMessage, ChatRequest, ChatResponse, ChatRole};

/// Result-cache lifetime: repeat calls within this window return the stored
/// analysis instead of spending provider tokens (T072 §3 step 1).
const CACHE_TTL_SECS: i64 = 86_400;
/// Generation headroom reserved out of the model window.
const RESERVED_TOKENS: usize = 500;
/// Output budget for one D2 verdict; the schema-bound JSON is small.
const MAX_OUTPUT_TOKENS: u32 = 2_048;
/// Token allowance for the account's role preamble when sizing the context
/// budget before the preamble itself is known.
const ROLE_PREAMBLE_TOKEN_ALLOWANCE: usize = 256;
/// D2 schema cap on `evidence` snippets (F_D2 §4.4), enforced defensively.
const EVIDENCE_MAX_CHARS: usize = 80;
/// F_D2 §3: the model suggests 1–3 next actions; extras are dropped.
const NEXT_ACTIONS_MAX: usize = 3;
/// `mails.spam_score` at or above this marks a likely mass marketing mail
/// (F_D2 §6, T072 §3 step 5).
const MARKETING_SPAM_SCORE: f64 = 0.7;

/// The D2 §4.4 output contract as a compact JSON Schema string, embedded in
/// the system prompt so the model has the exact shape to conform to.
pub const SALES_JSON_SCHEMA: &str = r#"{"type":"object","required":["counterparty_profile","needs_and_intents","concession_advice","next_actions"],"properties":{"counterparty_profile":{"type":"object","required":["stance","tone","authority_signal","observations"],"properties":{"stance":{"enum":["cooperative","neutral","adversarial"]},"tone":{"enum":["formal","casual"]},"authority_signal":{"type":"string"},"observations":{"type":"array","items":{"type":"string"}}}},"needs_and_intents":{"type":"array","items":{"type":"object","required":["need","priority","evidence"],"properties":{"need":{"type":"string"},"priority":{"enum":["high","medium","low"]},"evidence":{"type":"string","maxLength":80}}}},"concession_advice":{"type":"object","required":["concedable","negotiable","non_concedable"],"properties":{"concedable":{"type":"array","items":{"type":"string"}},"negotiable":{"type":"array","items":{"type":"string"}},"non_concedable":{"type":"array","items":{"type":"string"}}}},"next_actions":{"type":"array","minItems":1,"maxItems":3,"items":{"type":"object","required":["action","timeline"],"properties":{"action":{"type":"string"},"timeline":{"enum":["immediate","24h","72h","this_week"]}}}}}}"#;

/// Marketing-mail addendum (T072 §3 step 5): the model marks the mail instead
/// of the pipeline refusing the analysis.
const MARKETING_GUARD_NOTE: &str = "Note: This may be a mass marketing email. If so, output \
     counterparty_profile.stance='neutral' and add an observation noting this.";

/// The built-in sales-role prompt template (F_D2 §4.3, T072 §6). Ships with
/// the client and is not user-editable; only the stance intensity is a user
/// setting (`gentle` / `balanced` / `assertive`).
pub fn sales_system_prompt(role_description: &str, stance: &str) -> String {
    let stance_instruction = match stance {
        "gentle" => "Adopt a gentle, collaborative tone. Prioritise long-term relationship.",
        "assertive" => {
            "Adopt a confident, results-oriented stance. Push for our interests clearly."
        }
        // "balanced" and any unknown value fall back to the default.
        _ => "Adopt a balanced, professional tone.",
    };
    format!(
        "You are a senior international business consultant AI assistant. \
         Your position: assist OUR side in achieving a favourable outcome. {stance_instruction}\n\
         Role context: {role_description}\n\
         Safety: never fabricate commitments; flag uncertainty; \
         all recommendations are advisory only.\n\
         Output ONLY the JSON object, no markdown, no commentary.\n\
         Schema:\n{SALES_JSON_SCHEMA}"
    )
}

/// The D2 analysis pipeline. Borrows [`AppState`] for the duration of one
/// `run`; the IPC command (`commands::ai_roles`) is its only production caller.
pub struct SalesAnalysisPipeline<'a> {
    state: &'a AppState,
}

impl<'a> SalesAnalysisPipeline<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    /// Run (or replay from cache) the D2 sales analysis for one mail. See the
    /// module docs for the full pipeline.
    pub async fn run(&self, params: &AnalyzeSalesContextParams) -> AppResult<SalesAnalysisResult> {
        let db = self.state.storage.db().pool();

        // 1) Trigger mail → owning account, thread, counterparty columns, and
        // the marketing-detection inputs (T072 §3 steps 2 + 5).
        let mail_row = sqlx::query(
            "SELECT account_id, thread_id, from_name, from_email, to_addrs, spam_score \
             FROM mails WHERE id = ? AND is_deleted = 0",
        )
        .bind(&params.mail_id)
        .fetch_optional(db)
        .await
        .map_err(map_sqlx_err)?
        .ok_or(AppError::NotFound)?;
        let account_id: String = mail_row.get("account_id");
        let thread_id: Option<String> = mail_row.get("thread_id");
        let from_name: Option<String> = mail_row.get("from_name");
        let from_email: String = mail_row.get("from_email");
        let recipient_domain = first_recipient_domain(&mail_row.get::<String, _>("to_addrs"));
        let spam_score: Option<f64> = mail_row.get("spam_score");

        // 2) 24-hour cache (T072 §3 step 1): replay without touching the
        // provider.
        if !params.force_new {
            if let Some(cached) = self.cached_result(&params.mail_id).await? {
                tracing::info!(
                    event = "sales_analysis_cache_hit",
                    mail_id = %params.mail_id,
                    account_id = %account_id,
                    decision_id = %cached.decision_id,
                    "returning cached D2 analysis"
                );
                return Ok(cached);
            }
        }

        // 3) Provider + model (F4 matrix RiskReason row; enforces the daily
        // query limit before any network call).
        let client = self
            .state
            .ai
            .resolve(&account_id, Capability::RiskReason)
            .await?;
        let model = self
            .state
            .ai
            .account_config(&account_id)
            .await?
            .model
            .unwrap_or_default();

        // 4) Stance intensity from the account's style_profile JSON blob
        // (T072 §3 step 3; default "balanced").
        let stance_setting = self.sales_stance(&account_id).await?;

        // 5) Context budget: window − system-prompt estimate − reserved
        // headroom. An oversize target mail surfaces as AI_CONTEXT_TOO_LONG
        // from the assembler (D2 has no segmentation path — F_D2 §6 has no
        // oversize clause).
        let prompt_overhead =
            estimate_tokens(&sales_system_prompt("", "balanced")) + ROLE_PREAMBLE_TOKEN_ALLOWANCE;
        let token_budget = client
            .context_window()
            .saturating_sub(prompt_overhead + RESERVED_TOKENS);
        let mut ctx_params = RoleContextParams::new(
            params.mail_id.clone(),
            account_id.clone(),
            token_budget,
            Capability::RiskReason,
        );
        ctx_params.thread_id = thread_id;
        // Sales analysis goes through the shared engine (analysis/54 §4): the
        // anchored adapter returns the unified AssembledContext.
        let ctx = MailboxContextEngine::new(self.state)
            .assemble_for_mail(&ctx_params)
            .await?;
        let target = ctx.target().ok_or(AppError::NotFound)?;

        // 6) Prompt assembly (dev/06 §5 order: role > safety > GTE context >
        // contact history > target mail).
        let system = format!(
            "{}\n{}",
            sales_system_prompt(&ctx.role_preamble, &stance_setting),
            ctx.safety_preamble
        );
        let chunk_senders = self.chunk_senders(&ctx.knowledge_refs).await?;
        let mut user = build_grounding(
            &ctx,
            &chunk_senders,
            from_name.as_deref(),
            recipient_domain.as_deref(),
        );
        if let Some(history) = &ctx.contact_history {
            user.push_str(&format!(
                "[Contact history: {} interactions, {} replies]\n",
                history.data.interaction_count, history.data.reply_count
            ));
        }
        if is_marketing_mail(&from_email, spam_score) {
            user.push_str(MARKETING_GUARD_NOTE);
            user.push('\n');
        }
        user.push_str(&format!(
            "[Current Mail Subject: {}]\n{}",
            target.subject, target.content
        ));

        let request = ChatRequest {
            model,
            system,
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: user,
            }],
            max_tokens: MAX_OUTPUT_TOKENS,
            // Analysis reasoning is deterministic (dev/06 §2.1).
            temperature: 0.0,
            stop: Vec::new(),
            purpose: Capability::RiskReason,
            request_id: Uuid::new_v4(),
        };

        // 7) Provider call with strict validation and one re-prompt retry.
        let (output, response) = self
            .chat_validated(client.as_ref(), &request, &params.mail_id)
            .await?;
        let (counterparty_profile, needs_and_intents, concession_advice, next_actions) =
            output.into_parts();

        // 8) Contact-history snapshot for the wire result (None on first
        // contact — F_D2 §6).
        let contact_history = ctx.contact_history.as_ref().map(|h| ContactHistorySummary {
            interaction_count: h.data.interaction_count,
            reply_count: h.data.reply_count,
            style_notes: h
                .data
                .style_notes
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
        });

        let result = SalesAnalysisResult {
            decision_id: new_uuid(),
            mail_id: params.mail_id.clone(),
            account_id,
            counterparty_profile,
            needs_and_intents,
            concession_advice,
            next_actions,
            contact_history,
            ai_model: if response.model_echo.is_empty() {
                request.model.clone()
            } else {
                response.model_echo.clone()
            },
            knowledge_refs: ctx.knowledge_refs.clone(),
            created_at: now_unix(),
        };
        self.persist(
            &result,
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
            response.latency_ms.max(1),
        )
        .await?;

        // Identifiers, counts, and enum tags only — never content (dev/09 §5).
        tracing::info!(
            event = "sales_analysis_complete",
            mail_id = %result.mail_id,
            account_id = %result.account_id,
            decision_id = %result.decision_id,
            stance = result.counterparty_profile.stance.as_wire(),
            needs = result.needs_and_intents.len(),
            actions = result.next_actions.len(),
            knowledge_refs = result.knowledge_refs.len(),
            latency_ms = response.latency_ms,
            "sales D2 analysis complete"
        );
        Ok(result)
    }

    /// The freshest D2 audit row for this mail within the cache TTL, replayed
    /// from the result JSON stored in `action_description`. The
    /// `result_description LIKE 'D2 sales%'` predicate keeps D1 rows (and any
    /// other `impact = 'context'` writers) out of this cache (T072 §3 step 1).
    /// Rows that fail to parse fall through to a fresh run.
    async fn cached_result(&self, mail_id: &str) -> AppResult<Option<SalesAnalysisResult>> {
        let row = sqlx::query(
            "SELECT action_description FROM ai_decisions \
             WHERE mail_id = ? AND impact = 'context' \
                 AND result_description LIKE 'D2 sales%' AND created_at > ? \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(mail_id)
        .bind(now_unix() - CACHE_TTL_SECS)
        .fetch_optional(self.state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(row.and_then(|r| {
            serde_json::from_str::<SalesAnalysisResult>(&r.get::<String, _>("action_description"))
                .ok()
        }))
    }

    /// `sales_stance` from the account's `style_profile` JSON blob; `balanced`
    /// when the row, blob, or key is absent or unreadable (T072 §6).
    async fn sales_stance(&self, account_id: &str) -> AppResult<String> {
        let row = sqlx::query("SELECT style_profile FROM account_ai_settings WHERE account_id = ?")
            .bind(account_id)
            .fetch_optional(self.state.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(stance_from_style_profile(
            row.and_then(|r| r.get::<Option<String>, _>("style_profile"))
                .as_deref(),
        ))
    }

    /// `chat()` plus strict D2 validation with exactly one re-prompt retry
    /// (F_D2 §4.4). Provider transport errors propagate immediately — only an
    /// unparseable/non-conforming body earns the retry.
    async fn chat_validated(
        &self,
        client: &dyn AiProviderClient,
        request: &ChatRequest,
        mail_id: &str,
    ) -> AppResult<(ModelOutput, ChatResponse)> {
        let first = client.chat(request.clone()).await?;
        if let Some(output) = parse_d2_output(&first.text) {
            return Ok((output, first));
        }
        // Identifiers only: the invalid payload itself is never logged.
        tracing::warn!(
            event = "sales_analysis_invalid_output",
            mail_id = %mail_id,
            attempt = 1,
            "D2 output failed schema validation; retrying once"
        );
        let mut retry = request.clone();
        retry.request_id = Uuid::new_v4();
        let second = client.chat(retry).await?;
        match parse_d2_output(&second.text) {
            Some(output) => Ok((output, second)),
            None => {
                tracing::warn!(
                    event = "sales_analysis_invalid_output",
                    mail_id = %mail_id,
                    attempt = 2,
                    "D2 output invalid after retry; failing"
                );
                Err(AppError::Internal(anyhow::anyhow!(
                    "sales analysis output invalid after retry"
                )))
            }
        }
    }

    /// `from_email` per grounding mail, for the `[Prior Mail: …]` lines.
    async fn chunk_senders(&self, knowledge_refs: &[String]) -> AppResult<HashMap<String, String>> {
        if knowledge_refs.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = vec!["?"; knowledge_refs.len()].join(",");
        let sql = format!("SELECT id, from_email FROM mails WHERE id IN ({placeholders})");
        let mut query = sqlx::query(&sql);
        for mail_id in knowledge_refs {
            query = query.bind(mail_id);
        }
        let rows = query
            .fetch_all(self.state.storage.db().pool())
            .await
            .map_err(map_sqlx_err)?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("id"), r.get::<String, _>("from_email")))
            .collect())
    }

    /// The append-only `ai_decisions` audit row (T072 §3 step 8). D2 writes no
    /// `risk_events` (step 9), so a single insert is the whole persistence
    /// step.
    async fn persist(
        &self,
        result: &SalesAnalysisResult,
        input_tokens: u32,
        output_tokens: u32,
        latency_ms: u32,
    ) -> AppResult<()> {
        // The full result JSON backs the 24-hour cache (T072 §3 step 1).
        let payload = serde_json::to_string(result)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize analysis: {e}")))?;
        let knowledge_refs_json = serde_json::to_string(&result.knowledge_refs)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize knowledge refs: {e}")))?;

        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, mail_id, decision_type, impact, \
                 action_description, knowledge_refs, knowledge_summary, result_description, \
                 ai_model, input_tokens, output_tokens, latency_ms, created_at) \
             VALUES (?, ?, ?, 'draft_created', 'context', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&result.decision_id)
        .bind(&result.account_id)
        .bind(&result.mail_id)
        .bind(&payload)
        .bind(&knowledge_refs_json)
        .bind(format!(
            "Grounded on {} prior mails",
            result.knowledge_refs.len()
        ))
        // Content-free summary (dev/09 §5): enum tag and counts only. The
        // "D2 sales" prefix is the cache discriminator — keep them in sync.
        .bind(format!(
            "D2 sales analysis: stance={}, needs={}, actions={}",
            result.counterparty_profile.stance.as_wire(),
            result.needs_and_intents.len(),
            result.next_actions.len()
        ))
        .bind(&result.ai_model)
        .bind(input_tokens as i64)
        .bind(output_tokens as i64)
        .bind(latency_ms as i64)
        .bind(result.created_at)
        .execute(self.state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }
}

// ── Model-output parsing (D2 §4.4 wire shape: snake_case keys) ───────────────

/// Raw model output. Field names follow the D2 schema exactly (snake_case);
/// all four top-level fields are required, so a missing one fails validation.
/// The shared wire enums make unknown `stance`/`tone`/`priority`/`timeline`
/// tags a hard parse error (strict per F_D2 §4.4).
///
/// Deliberately no `Debug` derive: these structs carry mail excerpts, and the
/// logging red-line (dev/09 §5) is easier to uphold when they cannot be
/// formatted into a log line at all.
#[derive(serde::Deserialize)]
struct ModelOutput {
    counterparty_profile: ModelProfile,
    needs_and_intents: Vec<ModelNeed>,
    concession_advice: ModelConcessions,
    next_actions: Vec<ModelAction>,
}

#[derive(serde::Deserialize)]
struct ModelProfile {
    stance: CounterpartyStance,
    tone: CounterpartyTone,
    authority_signal: String,
    observations: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ModelNeed {
    need: String,
    priority: NeedPriority,
    evidence: String,
}

#[derive(serde::Deserialize)]
struct ModelConcessions {
    concedable: Vec<String>,
    negotiable: Vec<String>,
    non_concedable: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ModelAction {
    action: String,
    timeline: NextActionTimeline,
}

impl ModelOutput {
    /// Convert into wire shapes, enforcing the D2 caps defensively: `evidence`
    /// at 80 chars, `next_actions` at 3 entries (F_D2 §3, §4.4).
    fn into_parts(
        self,
    ) -> (
        CounterpartyProfile,
        Vec<NeedItem>,
        ConcessionAdvice,
        Vec<NextAction>,
    ) {
        let profile = CounterpartyProfile {
            stance: self.counterparty_profile.stance,
            tone: self.counterparty_profile.tone,
            authority_signal: self.counterparty_profile.authority_signal,
            observations: self.counterparty_profile.observations,
        };
        let needs = self
            .needs_and_intents
            .into_iter()
            .map(|n| NeedItem {
                need: n.need,
                priority: n.priority,
                evidence: truncate_chars(&n.evidence, EVIDENCE_MAX_CHARS),
            })
            .collect();
        let concessions = ConcessionAdvice {
            concedable: self.concession_advice.concedable,
            negotiable: self.concession_advice.negotiable,
            non_concedable: self.concession_advice.non_concedable,
        };
        let mut actions: Vec<NextAction> = self
            .next_actions
            .into_iter()
            .map(|a| NextAction {
                action: a.action,
                timeline: a.timeline,
            })
            .collect();
        actions.truncate(NEXT_ACTIONS_MAX);
        (profile, needs, concessions, actions)
    }
}

/// Extract and strictly parse the D2 JSON object from a completion. Tolerates
/// stray prose/fences around the object (first `{` to last `}`), but nothing
/// inside it: missing fields, unknown enum tags, and an empty `next_actions`
/// array fail (F_D2 §3 mandates 1–3 actions). Returns `None` on any failure so
/// no fragment of the payload can travel on an error value.
fn parse_d2_output(raw: &str) -> Option<ModelOutput> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    let output = serde_json::from_str::<ModelOutput>(&raw[start..=end]).ok()?;
    if output.next_actions.is_empty() {
        return None;
    }
    Some(output)
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// `sales_stance` out of the `style_profile` JSON blob (T072 §6). Any missing
/// or unreadable layer degrades to `"balanced"` — a corrupt blob must never
/// fail the analysis.
fn stance_from_style_profile(style_profile: Option<&str>) -> String {
    style_profile
        .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
        .and_then(|v| {
            v.get("sales_stance")
                .and_then(|s| s.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "balanced".to_owned())
}

/// Marketing-mail heuristic (T072 §3 step 5): a high spam score or a bulk
/// sender address pattern. Detection only flavors the prompt — the analysis
/// still runs (F_D2 §6).
fn is_marketing_mail(from_email: &str, spam_score: Option<f64>) -> bool {
    if spam_score.is_some_and(|s| s >= MARKETING_SPAM_SCORE) {
        return true;
    }
    let sender = from_email.to_lowercase();
    sender.contains("noreply") || sender.contains("no-reply") || sender.contains("unsubscribe")
}

/// Grounding block prepended to the user message (dev/06 §5 order inside the
/// user turn: parties, thread, GTE context; contact history and the current
/// mail body follow the block).
fn build_grounding(
    ctx: &AssembledContext,
    chunk_senders: &HashMap<String, String>,
    from_name: Option<&str>,
    recipient_domain: Option<&str>,
) -> String {
    let mut block = String::new();
    let target_from = ctx.target().map(|t| t.from_email.as_str()).unwrap_or("");
    let from = match from_name {
        Some(name) if !name.trim().is_empty() => {
            format!("{} <{}>", name.trim(), target_from)
        }
        _ => format!("<{target_from}>"),
    };
    block.push_str(&format!("[Parties: from {from}"));
    if let Some(domain) = recipient_domain {
        block.push_str(&format!("; recipient domain {domain}"));
    }
    block.push_str("]\n");
    for mail in ctx.items_of(ContextItemKind::Thread) {
        block.push_str(&format!(
            "[Thread Mail: {} from {}: {}]\n",
            format_date(mail.date_sent),
            mail.from_email,
            mail.content
        ));
    }
    for chunk in ctx.items_of(ContextItemKind::Semantic) {
        let sender = chunk_senders
            .get(&chunk.mail_id)
            .map(String::as_str)
            .unwrap_or("unknown");
        block.push_str(&format!(
            "[Prior Mail: {} from {}: {}]\n",
            format_date(chunk.date_sent),
            sender,
            chunk.content
        ));
    }
    block
}

/// Domain of the first recipient in the `to_addrs` JSON array (the "both
/// parties" input of F_D2 §4.2).
fn first_recipient_domain(to_addrs_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(to_addrs_json).ok()?;
    let email = value.get(0)?.get("email")?.as_str()?;
    let (_, domain) = email.split_once('@')?;
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_lowercase())
    }
}

/// `YYYY-MM-DD` for prompt context lines; falls back to the raw timestamp.
fn format_date(unix_secs: i64) -> String {
    chrono::DateTime::from_timestamp(unix_secs, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| unix_secs.to_string())
}

/// Conservative token estimate, same heuristic as the context packer
/// (1 token ≈ 4 bytes, never zero).
fn estimate_tokens(s: &str) -> usize {
    (s.len() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::{ChatDeltaStream, ProviderError};
    use crate::ai::types::{FinishReason, ProviderHealth, TokenUsage};
    use crate::error::IpcError;
    use crate::types::{AiProvider, ErrorCode};
    use crate::util::now_unix;
    use crate::vector::VectorRow;

    // ── Seeding helpers ──────────────────────────────────────────────────────

    /// Account with the sales role + an `account_ai_settings` row routed to
    /// the (mock) OpenAI provider, optionally carrying a `style_profile` blob.
    async fn seed_account(state: &AppState, id: &str, style_profile: Option<&str>) {
        let pool = state.storage.db().pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, 'Sales Desk', 'imap', 'slate', 'S', 'sales', \
                 'Negotiate inbound deals and protect our margins.', 0, 0)",
        )
        .bind(id)
        .bind(format!("{id}@corp.com"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, \
                 daily_query_limit, style_profile, updated_at) \
             VALUES (?, 1, 'openai', 'gpt-test', 50, ?, 0)",
        )
        .bind(id)
        .bind(style_profile)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_mail(
        state: &AppState,
        id: &str,
        acc: &str,
        from_email: &str,
        body: &str,
        spam_score: Option<f64>,
        date_sent: i64,
    ) {
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_name, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, spam_score, \
                 embedding_status, created_at, updated_at) \
             VALUES (?, ?, ?, 'Q3 reorder pricing and volume discount', 'Dana Webb', ?, \
                 '[{\"name\":\"Sales Desk\",\"email\":\"sales@corp.com\"}]', ?, ?, ?, ?, ?, \
                 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(format!("<{id}@x>"))
        .bind(from_email)
        .bind(date_sent)
        .bind(date_sent)
        .bind(body)
        .bind(truncate_chars(body, 200))
        .bind(spam_score)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// Embed `text` and upsert it as the mail's chunk in the vector store, so
    /// GTE retrieval has something to ground on (mirrors legal.rs tests).
    async fn index_mail(state: &AppState, id: &str, acc: &str, text: &str) {
        let rows = vec![VectorRow {
            chunk_id: format!("{id}:0"),
            mail_id: id.into(),
            chunk_index: 0,
            account_id: acc.into(),
            from_email: BUYER_EMAIL.into(),
            date_sent: now_unix(),
            subject: text.into(),
            snippet: text.into(),
            embedding_model: "bge-m3".into(),
            vector: state.embedder.embed(text).unwrap(),
        }];
        state.storage.vectors().upsert(&rows).unwrap();
    }

    /// Insert a contacts row for the counterparty.
    async fn seed_contact(
        state: &AppState,
        email: &str,
        interactions: i64,
        replies: i64,
        style_notes: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO contacts (id, email, first_seen_at, last_seen_at, \
                 interaction_count, reply_count, style_notes, created_at, updated_at) \
             VALUES (?, ?, 0, 0, ?, ?, ?, 0, 0)",
        )
        .bind(new_uuid())
        .bind(email)
        .bind(interactions)
        .bind(replies)
        .bind(style_notes)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    const BUYER_EMAIL: &str = "buyer@client.com";
    const TRIGGER_BODY: &str =
        "the quarterly reorder pricing for five thousand units and the volume discount request";

    /// Account + trigger mail + two semantically related, indexed prior mails
    /// so `knowledge_refs` is non-empty, plus a contacts row for the buyer.
    async fn seed_corpus(state: &AppState, acc: &str) {
        seed_account(state, acc, None).await;
        seed_mail(
            state,
            "trigger",
            acc,
            BUYER_EMAIL,
            TRIGGER_BODY,
            None,
            now_unix(),
        )
        .await;
        index_mail(state, "trigger", acc, TRIGGER_BODY).await;
        let related1 = "prior reorder discussed the quarterly pricing and a volume discount";
        let related2 = "the volume discount request from last quarter reorder pricing";
        seed_mail(
            state,
            "k1",
            acc,
            BUYER_EMAIL,
            related1,
            None,
            now_unix() - 100,
        )
        .await;
        index_mail(state, "k1", acc, related1).await;
        seed_mail(
            state,
            "k2",
            acc,
            BUYER_EMAIL,
            related2,
            None,
            now_unix() - 200,
        )
        .await;
        index_mail(state, "k2", acc, related2).await;
        seed_contact(state, BUYER_EMAIL, 17, 9, Some(r#"{"greeting":"Hi team"}"#)).await;
    }

    fn register_mock(state: &AppState) -> Arc<MockProvider> {
        let mock = Arc::new(MockProvider::healthy(AiProvider::Openai));
        state.ai.register(mock.clone());
        mock
    }

    fn response(text: impl Into<String>) -> ChatResponse {
        ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage {
                prompt_tokens: 420,
                completion_tokens: 96,
            },
            model_echo: "gpt-test-echo".into(),
            latency_ms: 7,
        }
    }

    const HIGH_EVIDENCE: &str = "we expect a better rate at five thousand units";

    /// A schema-conforming D2 verdict: cooperative formal buyer, two needs,
    /// three-tier concessions, two next actions.
    fn d2_json() -> String {
        serde_json::json!({
            "counterparty_profile": {
                "stance": "cooperative",
                "tone": "formal",
                "authority_signal": "Signs as procurement director",
                "observations": [
                    "References last quarter's order volume",
                    "Asks for a decision this week"
                ]
            },
            "needs_and_intents": [
                {
                    "need": "Volume discount on the Q3 reorder",
                    "priority": "high",
                    "evidence": HIGH_EVIDENCE
                },
                {
                    "need": "Earlier delivery window",
                    "priority": "medium",
                    "evidence": "ideally landed before the trade fair"
                }
            ],
            "concession_advice": {
                "concedable": ["Three percent discount above five thousand units"],
                "negotiable": ["Freight terms", "Payment schedule"],
                "non_concedable": ["Unit price below the cost floor"]
            },
            "next_actions": [
                { "action": "Send a revised quote with tiered pricing", "timeline": "24h" },
                { "action": "Propose a call to lock the delivery window", "timeline": "this_week" }
            ]
        })
        .to_string()
    }

    async fn analyze(
        state: &AppState,
        mail_id: &str,
        force_new: bool,
    ) -> AppResult<SalesAnalysisResult> {
        SalesAnalysisPipeline::new(state)
            .run(&AnalyzeSalesContextParams {
                mail_id: mail_id.into(),
                force_new,
            })
            .await
    }

    /// Wraps a [`MockProvider`] and records every `chat()` request's system
    /// and user text, so prompt-content assertions (stance instruction,
    /// marketing guard) can run end-to-end without touching the mock seam.
    struct CapturingProvider {
        inner: MockProvider,
        /// `(system, joined user contents)` per `chat()` call.
        requests: Mutex<Vec<(String, String)>>,
    }

    impl CapturingProvider {
        fn new(inner: MockProvider) -> Self {
            Self {
                inner,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn captured(&self) -> Vec<(String, String)> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl AiProviderClient for CapturingProvider {
        async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            let user = req
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            self.requests
                .lock()
                .unwrap()
                .push((req.system.clone(), user));
            self.inner.chat(req).await
        }

        async fn chat_stream(&self, req: ChatRequest) -> Result<ChatDeltaStream, ProviderError> {
            self.inner.chat_stream(req).await
        }

        async fn health(&self) -> Result<ProviderHealth, ProviderError> {
            self.inner.health().await
        }

        fn id(&self) -> AiProvider {
            self.inner.id()
        }

        fn context_window(&self) -> usize {
            self.inner.context_window()
        }
    }

    fn register_capturing(state: &AppState) -> Arc<CapturingProvider> {
        let capturing = Arc::new(CapturingProvider::new(MockProvider::healthy(
            AiProvider::Openai,
        )));
        state.ai.register(capturing.clone());
        capturing
    }

    // ── Pipeline integration tests ───────────────────────────────────────────

    #[tokio::test]
    async fn success_returns_result_and_writes_audit_but_no_risk_rows() {
        let (state, _rx) = AppState::test_state().await;
        seed_corpus(&state, "acct").await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response(d2_json())));

        let result = analyze(&state, "trigger", true).await.unwrap();
        assert_eq!(result.mail_id, "trigger");
        assert_eq!(result.account_id, "acct");
        assert_eq!(
            result.counterparty_profile.stance,
            CounterpartyStance::Cooperative
        );
        assert_eq!(result.counterparty_profile.tone, CounterpartyTone::Formal);
        assert_eq!(result.counterparty_profile.observations.len(), 2);
        assert_eq!(result.needs_and_intents.len(), 2);
        assert_eq!(result.needs_and_intents[0].priority, NeedPriority::High);
        assert_eq!(result.concession_advice.negotiable.len(), 2);
        assert_eq!(result.next_actions.len(), 2);
        assert_eq!(
            result.next_actions[0].timeline,
            NextActionTimeline::Within24h
        );
        assert_eq!(
            result.next_actions[1].timeline,
            NextActionTimeline::ThisWeek
        );
        assert!(!result.knowledge_refs.is_empty(), "grounded on prior mails");
        assert_eq!(result.ai_model, "gpt-test-echo");

        // Contact history travels on the result (F_D2 §4.2).
        let history = result.contact_history.as_ref().expect("contact row exists");
        assert_eq!(history.interaction_count, 17);
        assert_eq!(history.reply_count, 9);
        assert_eq!(
            history.style_notes,
            Some(serde_json::json!({"greeting": "Hi team"}))
        );

        // Audit row (dev/06 §9, T072 §3 step 8).
        let pool = state.storage.db().pool();
        let row = sqlx::query(
            "SELECT decision_type, impact, knowledge_refs, ai_model, input_tokens, \
                 output_tokens, latency_ms FROM ai_decisions WHERE id = ?",
        )
        .bind(&result.decision_id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("decision_type"), "draft_created");
        assert_eq!(row.get::<String, _>("impact"), "context");
        let refs: Vec<String> =
            serde_json::from_str(&row.get::<String, _>("knowledge_refs")).unwrap();
        assert!(!refs.is_empty(), "knowledge_refs is a non-empty JSON array");
        assert_eq!(refs, result.knowledge_refs);
        assert!(row.get::<i64, _>("latency_ms") > 0);
        assert!(row.get::<i64, _>("input_tokens") > 0);
        assert!(row.get::<i64, _>("output_tokens") > 0);

        // T072 §3 step 9: D2 never writes risk events.
        let (risks,): (i64,) = sqlx::query_as("SELECT count(*) FROM risk_events")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(risks, 0, "D2 is context assistance, not a safety risk");
    }

    #[tokio::test]
    async fn cache_hit_skips_provider_and_force_new_bypasses_cache() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            BUYER_EMAIL,
            "reorder pricing question",
            None,
            now_unix(),
        )
        .await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response(d2_json())));

        let first = analyze(&state, "m1", false).await.unwrap();
        assert_eq!(mock.chat_call_count(), 1);

        // Within 24h, force_new = false replays the stored verdict.
        let cached = analyze(&state, "m1", false).await.unwrap();
        assert_eq!(mock.chat_call_count(), 1, "no provider call on cache hit");
        assert_eq!(cached, first);

        // force_new = true ignores the cache and produces a new decision row.
        mock.push_chat(Ok(response(d2_json())));
        let fresh = analyze(&state, "m1", true).await.unwrap();
        assert_eq!(mock.chat_call_count(), 2);
        assert_ne!(fresh.decision_id, first.decision_id);
    }

    #[tokio::test]
    async fn first_contact_returns_null_history_without_error() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        // No contacts row for this sender (first contact — F_D2 §6).
        seed_mail(
            &state,
            "m1",
            "acct",
            "newlead@startup.io",
            "intro and pricing ask",
            None,
            now_unix(),
        )
        .await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response(d2_json())));

        let result = analyze(&state, "m1", true).await.unwrap();
        assert!(result.contact_history.is_none(), "null on first contact");
        assert_eq!(result.next_actions.len(), 2);
    }

    #[tokio::test]
    async fn assertive_stance_reaches_system_prompt() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", Some(r#"{"sales_stance":"assertive"}"#)).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            BUYER_EMAIL,
            "contract pricing pushback",
            None,
            now_unix(),
        )
        .await;
        let capturing = register_capturing(&state);
        capturing.inner.push_chat(Ok(response(d2_json())));

        analyze(&state, "m1", true).await.unwrap();

        let requests = capturing.captured();
        assert_eq!(requests.len(), 1);
        let (system, _user) = &requests[0];
        assert!(
            system.contains("confident, results-oriented"),
            "assertive stance instruction must reach the provider"
        );
        assert!(system.contains(SALES_JSON_SCHEMA));
    }

    #[tokio::test]
    async fn marketing_sender_injects_guard_note() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            "noreply@deals.example.com",
            "limited time wholesale offer",
            None,
            now_unix(),
        )
        .await;
        let capturing = register_capturing(&state);
        capturing.inner.push_chat(Ok(response(d2_json())));

        analyze(&state, "m1", true).await.unwrap();

        let requests = capturing.captured();
        let (_system, user) = &requests[0];
        assert!(user.contains(MARKETING_GUARD_NOTE), "guard note in prompt");
    }

    #[tokio::test]
    async fn contact_history_line_reaches_prompt() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            BUYER_EMAIL,
            "follow-up on the proposal",
            None,
            now_unix(),
        )
        .await;
        seed_contact(&state, BUYER_EMAIL, 12, 7, None).await;
        let capturing = register_capturing(&state);
        capturing.inner.push_chat(Ok(response(d2_json())));

        analyze(&state, "m1", true).await.unwrap();

        let (_system, user) = &capturing.captured()[0];
        assert!(user.contains("[Contact history: 12 interactions, 7 replies]"));
    }

    #[tokio::test]
    async fn invalid_json_retries_once_then_succeeds() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            BUYER_EMAIL,
            "negotiation thread",
            None,
            now_unix(),
        )
        .await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response("the buyer seems cooperative overall")));
        mock.push_chat(Ok(response(d2_json())));

        let result = analyze(&state, "m1", true).await.unwrap();
        assert_eq!(mock.chat_call_count(), 2, "exactly one retry");
        assert_eq!(
            result.counterparty_profile.stance,
            CounterpartyStance::Cooperative
        );
    }

    #[tokio::test]
    async fn invalid_json_twice_returns_internal_and_persists_nothing() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            BUYER_EMAIL,
            "negotiation thread",
            None,
            now_unix(),
        )
        .await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response("no json at all")));
        mock.push_chat(Ok(response(
            "{\"counterparty_profile\": \"not an object\"}",
        )));

        let err = analyze(&state, "m1", true).await.unwrap_err();
        let ipc: IpcError = err.into();
        assert_eq!(ipc.code, ErrorCode::Internal);
        assert_eq!(mock.chat_call_count(), 2);

        let pool = state.storage.db().pool();
        let (decisions,): (i64,) = sqlx::query_as("SELECT count(*) FROM ai_decisions")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(decisions, 0, "failed runs leave no rows");
    }

    #[tokio::test]
    async fn provider_unreachable_propagates_without_retry() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            BUYER_EMAIL,
            "negotiation thread",
            None,
            now_unix(),
        )
        .await;
        let mock = register_mock(&state);
        mock.push_chat(Err(ProviderError::Unreachable("dns failure".into())));

        let err = analyze(&state, "m1", true).await.unwrap_err();
        let ipc: IpcError = err.into();
        assert_eq!(ipc.code, ErrorCode::AiProviderUnreachable);
        assert_eq!(mock.chat_call_count(), 1, "transport errors never retry");
    }

    #[tokio::test]
    async fn unknown_mail_is_not_found() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        register_mock(&state);
        let err = analyze(&state, "missing", true).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    /// dev/09 §5: the audit summary carries enum tags and counts only — no
    /// body text, no evidence snippet, no advice copy.
    #[tokio::test]
    async fn audit_summary_is_content_free() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct", None).await;
        seed_mail(
            &state,
            "m1",
            "acct",
            BUYER_EMAIL,
            TRIGGER_BODY,
            None,
            now_unix(),
        )
        .await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response(d2_json())));

        analyze(&state, "m1", true).await.unwrap();

        let summary: String =
            sqlx::query("SELECT result_description FROM ai_decisions WHERE mail_id = 'm1'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap()
                .get("result_description");
        assert_eq!(
            summary,
            "D2 sales analysis: stance=cooperative, needs=2, actions=2"
        );
        assert!(!summary.contains(TRIGGER_BODY));
        assert!(!summary.contains(HIGH_EVIDENCE));
        assert!(
            summary.starts_with("D2 sales"),
            "cache discriminator prefix"
        );
    }

    // ── Pure-helper unit tests ───────────────────────────────────────────────

    #[test]
    fn stance_reads_style_profile_and_degrades_to_balanced() {
        assert_eq!(
            stance_from_style_profile(Some(r#"{"sales_stance":"assertive"}"#)),
            "assertive"
        );
        assert_eq!(
            stance_from_style_profile(Some(r#"{"sales_stance":"gentle","greeting":"Hi"}"#)),
            "gentle"
        );
        // Missing key, corrupt JSON, and no blob at all → balanced.
        assert_eq!(
            stance_from_style_profile(Some(r#"{"greeting":"Hi"}"#)),
            "balanced"
        );
        assert_eq!(stance_from_style_profile(Some("{broken")), "balanced");
        assert_eq!(stance_from_style_profile(None), "balanced");
    }

    #[test]
    fn system_prompt_embeds_role_stance_and_schema() {
        let prompt = sales_system_prompt(
            "You are the sales assistant for sales@corp.com.",
            "balanced",
        );
        assert!(prompt.contains("senior international business consultant"));
        assert!(prompt.contains("sales@corp.com"));
        assert!(prompt.contains("balanced, professional tone"));
        assert!(prompt.contains(SALES_JSON_SCHEMA));
        assert!(prompt.contains("Output ONLY the JSON object"));

        assert!(sales_system_prompt("", "gentle").contains("gentle, collaborative tone"));
        assert!(sales_system_prompt("", "assertive").contains("confident, results-oriented"));
        // Unknown intensity values degrade to the balanced instruction.
        assert!(sales_system_prompt("", "ruthless").contains("balanced, professional tone"));
    }

    #[test]
    fn marketing_detection_matches_card_rules() {
        assert!(is_marketing_mail("noreply@shop.com", None));
        assert!(is_marketing_mail("no-reply@shop.com", None));
        assert!(is_marketing_mail("unsubscribe-list@shop.com", None));
        assert!(is_marketing_mail("dana@partner.com", Some(0.7)));
        assert!(is_marketing_mail("dana@partner.com", Some(0.95)));
        assert!(!is_marketing_mail("dana@partner.com", Some(0.69)));
        assert!(!is_marketing_mail("dana@partner.com", None));
    }

    #[test]
    fn parse_tolerates_fences_but_rejects_bad_shapes() {
        let wrapped = format!("```json\n{}\n```", d2_json());
        assert!(parse_d2_output(&wrapped).is_some());
        assert!(parse_d2_output(&d2_json()).is_some());
        assert!(parse_d2_output("plain prose, no object").is_none());
        // Missing required top-level field.
        assert!(parse_d2_output(
            r#"{"counterparty_profile":{"stance":"neutral","tone":"formal","authority_signal":"","observations":[]},"needs_and_intents":[],"concession_advice":{"concedable":[],"negotiable":[],"non_concedable":[]}}"#
        )
        .is_none());
        // Unknown stance tag.
        let bad_stance = d2_json().replace("\"cooperative\"", "\"friendly\"");
        assert!(parse_d2_output(&bad_stance).is_none());
        // Unknown timeline tag.
        let bad_timeline = d2_json().replace("\"24h\"", "\"next_month\"");
        assert!(parse_d2_output(&bad_timeline).is_none());
        // Empty next_actions violates the 1–3 actions contract (F_D2 §3).
        let no_actions = serde_json::json!({
            "counterparty_profile": {
                "stance": "neutral", "tone": "casual",
                "authority_signal": "", "observations": []
            },
            "needs_and_intents": [],
            "concession_advice": {"concedable": [], "negotiable": [], "non_concedable": []},
            "next_actions": []
        })
        .to_string();
        assert!(parse_d2_output(&no_actions).is_none());
    }

    #[test]
    fn parse_caps_evidence_length_and_action_count() {
        let long = "x".repeat(500);
        let raw = serde_json::json!({
            "counterparty_profile": {
                "stance": "adversarial", "tone": "casual",
                "authority_signal": "Unclear", "observations": []
            },
            "needs_and_intents": [
                {"need": "Lower price", "priority": "low", "evidence": long}
            ],
            "concession_advice": {"concedable": [], "negotiable": [], "non_concedable": []},
            "next_actions": [
                {"action": "a1", "timeline": "immediate"},
                {"action": "a2", "timeline": "24h"},
                {"action": "a3", "timeline": "72h"},
                {"action": "a4", "timeline": "this_week"}
            ]
        })
        .to_string();
        let (profile, needs, _, actions) = parse_d2_output(&raw).unwrap().into_parts();
        assert_eq!(profile.stance, CounterpartyStance::Adversarial);
        assert_eq!(needs[0].evidence.chars().count(), EVIDENCE_MAX_CHARS);
        assert_eq!(actions.len(), NEXT_ACTIONS_MAX, "extra actions are dropped");
    }

    #[test]
    fn first_recipient_domain_parses_to_addrs_json() {
        assert_eq!(
            first_recipient_domain(r#"[{"name":"Sales","email":"sales@Corp.COM"}]"#).as_deref(),
            Some("corp.com")
        );
        assert_eq!(first_recipient_domain("[]"), None);
        assert_eq!(first_recipient_domain("not json"), None);
        assert_eq!(
            first_recipient_domain(r#"[{"name":"x","email":"no-at-sign"}]"#),
            None
        );
    }
}
