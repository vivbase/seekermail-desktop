//! Legal-role analysis pipeline — D1 Legal Audit Assistant (T070, F_D1).
//!
//! [`LegalAnalysisPipeline::run`] turns one mail id into a structured
//! [`LegalAnalysisResult`]:
//!
//! 1. **24-hour cache** (`force_new = false`): the most recent
//!    `ai_decisions` D1 row within 24 h is returned as-is — no provider call,
//!    no `daily_query_limit` spend (F_D1 §4.5).
//! 2. **Context** via T074 [`assemble_role_context`] with
//!    `Capability::RiskReason` (role preamble, safety preamble, thread
//!    snippets, GTE chunks, `knowledge_refs`).
//! 3. **Provider call** through `AiRegistry::resolve(account, RiskReason)` —
//!    non-streaming `chat()` at `temperature = 0.0` (dev/06 §2.1, §4: risk
//!    verdicts are atomic).
//! 4. **Strict JSON validation** against the D1 schema (F_D1 §4.4): unknown
//!    `level`/`type` values and missing fields are rejected; one re-prompt
//!    retry, then `INTERNAL` ("output invalid after retry", F_D1 §6).
//! 5. **Oversize mails** (> 80 K chars ≈ 20 K tokens): the body is split on
//!    paragraph boundaries, the provider is called once per segment, and the
//!    verdicts are merged client-side (union of `risk_list` deduplicated by
//!    `original_text`; `key_clauses` from the last segment; first three
//!    `compliance_advice` entries — F_D1 §6, T070 §3 step 3).
//! 6. **Persistence** (one transaction): an append-only `ai_decisions` audit
//!    row (`impact = 'risk'`, `decision_type = risk_alert_t4/t3/t1` by overall
//!    level, `knowledge_refs`, token/latency accounting, and the full result
//!    JSON in `action_description` so the 24-hour cache can replay it), plus
//!    one `risk_events` row per high/medium item (high → level 4, never
//!    expires; medium → level 3, expires in 7 days; low items write nothing).
//!
//! **Privacy red-lines (T070 §6, dev/09 §5):** `risk_events.evidence` stores a
//! SHA-256 prefix of the flagged excerpt — never the excerpt itself — and this
//! module logs identifiers, counts, and levels only: never `body_text`,
//! `original_text`, or any other mail or model content.

use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::map_sqlx_err;
use crate::types::{
    AnalyzeLegalRiskParams, LegalAnalysisResult, LegalKeyClauses, LegalOverallLevel, LegalRiskItem,
    LegalRiskLevel, LegalRiskType,
};
use crate::util::{new_uuid, now_unix, truncate_chars};

use super::context::{assemble_role_context, RoleContext, RoleContextParams};
use super::provider::AiProviderClient;
use super::types::{Capability, ChatMessage, ChatRequest, ChatResponse, ChatRole};

/// Result-cache lifetime: repeat calls within this window return the stored
/// analysis instead of spending provider tokens (F_D1 §4.5).
const CACHE_TTL_SECS: i64 = 86_400;
/// `risk_events.expires_at` horizon for medium-level items (T070 §3 step 8).
/// High-level (T4) events never expire.
const MEDIUM_RISK_TTL_SECS: i64 = 7 * 86_400;
/// Body length above which the mail is analyzed in segments (~20 K tokens,
/// F_D1 §6 / T070 §3 step 3).
const SEGMENT_CHAR_LIMIT: usize = 80_000;
/// Generation headroom reserved out of the model window (T070 §2).
const RESERVED_TOKENS: usize = 500;
/// Output budget for one D1 verdict; the schema-bound JSON is small.
const MAX_OUTPUT_TOKENS: u32 = 2_048;
/// Token allowance for the account's role preamble when sizing the context
/// budget before the preamble itself is known.
const ROLE_PREAMBLE_TOKEN_ALLOWANCE: usize = 256;
/// D1 schema caps (F_D1 §4.4), enforced defensively on model output.
const ORIGINAL_TEXT_MAX_CHARS: usize = 120;
const FINDING_MAX_CHARS: usize = 80;
const SUGGESTION_MAX_CHARS: usize = 80;
const COMPLIANCE_ADVICE_MAX: usize = 3;
/// Hex chars kept of the excerpt hash (first 8 bytes of SHA-256, T070 §6).
const EVIDENCE_HASH_HEX_CHARS: usize = 16;

/// The D1 §4.4 output contract as a compact JSON Schema string, embedded in
/// the system prompt so the model has the exact shape to conform to.
pub const LEGAL_JSON_SCHEMA: &str = r#"{"type":"object","required":["risk_list","key_clauses","compliance_advice"],"properties":{"risk_list":{"type":"array","items":{"type":"object","required":["level","type","original_text","finding","suggestion"],"properties":{"level":{"enum":["high","medium","low"]},"type":{"enum":["payment","delivery","liability","confidentiality","dispute","other"]},"original_text":{"type":"string","maxLength":120},"finding":{"type":"string","maxLength":80},"suggestion":{"type":"string","maxLength":80}}}},"key_clauses":{"type":"object","properties":{"payment":{"type":"string"},"delivery":{"type":"string"},"liability":{"type":"string"},"confidentiality":{"type":"string"},"dispute_resolution":{"type":"string"}}},"compliance_advice":{"type":"array","maxItems":3,"items":{"type":"string"}}}}"#;

/// The built-in legal-role prompt template (F_D1 §4.3, T070 §6). Ships with
/// the client and is not user-editable.
pub fn legal_system_prompt(role_description: &str) -> String {
    format!(
        "You are a senior corporate legal counsel AI assistant. \
         Your task: analyse the email(s) provided and return a JSON object \
         that STRICTLY conforms to the schema below. \
         Role context: {role_description}\n\
         Safety: never fabricate legal commitments; flag uncertainty; \
         defer to human counsel for formal legal opinions.\n\
         Output ONLY the JSON object, no markdown, no commentary.\n\
         Schema:\n{LEGAL_JSON_SCHEMA}"
    )
}

/// The D1 analysis pipeline. Borrows [`AppState`] for the duration of one
/// `run`; the IPC command (`commands::ai_roles`) is its only production caller.
pub struct LegalAnalysisPipeline<'a> {
    state: &'a AppState,
}

impl<'a> LegalAnalysisPipeline<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    /// Run (or replay from cache) the D1 legal analysis for one mail. See the
    /// module docs for the full pipeline.
    pub async fn run(&self, params: &AnalyzeLegalRiskParams) -> AppResult<LegalAnalysisResult> {
        let db = self.state.storage.db().pool();

        // 1) Trigger mail → owning account, thread, counterparty columns.
        let mail_row = sqlx::query(
            "SELECT account_id, thread_id, from_name, to_addrs, \
                 length(COALESCE(body_text, '')) AS body_len \
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
        let recipient_domain = first_recipient_domain(&mail_row.get::<String, _>("to_addrs"));
        let body_len = mail_row.get::<i64, _>("body_len").max(0) as usize;

        // 2) 24-hour cache (F_D1 §4.5): replay without touching the provider.
        if !params.force_new {
            if let Some(cached) = self.cached_result(&params.mail_id).await? {
                tracing::info!(
                    event = "legal_analysis_cache_hit",
                    mail_id = %params.mail_id,
                    account_id = %account_id,
                    decision_id = %cached.decision_id,
                    "returning cached D1 analysis"
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

        // 4) Context budget: window − system-prompt estimate − reserved
        // headroom (T070 §2). For oversize mails the assembly budget is
        // inflated by the full body cost — the packer must admit the
        // untruncated target mail (dev/06 §5) even though each provider call
        // only ever carries one segment of it.
        let prompt_overhead =
            estimate_tokens(&legal_system_prompt("")) + ROLE_PREAMBLE_TOKEN_ALLOWANCE;
        let mut token_budget = client
            .context_window()
            .saturating_sub(prompt_overhead + RESERVED_TOKENS);
        if body_len > SEGMENT_CHAR_LIMIT {
            token_budget += body_len / 4 + 1;
        }
        let mut ctx_params = RoleContextParams::new(
            params.mail_id.clone(),
            account_id.clone(),
            token_budget,
            Capability::RiskReason,
        );
        ctx_params.thread_id = thread_id;
        let ctx = assemble_role_context(self.state, &ctx_params).await?;

        // 5) Prompt assembly (dev/06 §5 order: role > safety > GTE context >
        // target mail) and the per-segment provider loop.
        let system = format!(
            "{}\n{}",
            legal_system_prompt(&ctx.role_preamble),
            ctx.safety_preamble
        );
        let chunk_senders = self.chunk_senders(&ctx).await?;
        let grounding = build_grounding(
            &ctx,
            &chunk_senders,
            from_name.as_deref(),
            recipient_domain.as_deref(),
        );
        let segments = split_segments(&ctx.target_mail.body, SEGMENT_CHAR_LIMIT);
        let segment_count = segments.len();

        let mut risk_list: Vec<LegalRiskItem> = Vec::new();
        let mut seen_excerpts: HashSet<String> = HashSet::new();
        let mut key_clauses = LegalKeyClauses::default();
        let mut compliance_advice: Vec<String> = Vec::new();
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut latency_ms: u32 = 0;
        let mut model_echo = model.clone();

        for segment in &segments {
            let user = format!(
                "{grounding}[Current Mail Subject: {}]\n{}",
                ctx.target_mail.subject, segment
            );
            let request = ChatRequest {
                model: model.clone(),
                system: system.clone(),
                messages: vec![ChatMessage {
                    role: ChatRole::User,
                    content: user,
                }],
                max_tokens: MAX_OUTPUT_TOKENS,
                // Risk reasoning is deterministic (dev/06 §2.1).
                temperature: 0.0,
                stop: Vec::new(),
                purpose: Capability::RiskReason,
                request_id: Uuid::new_v4(),
            };
            let (output, response) = self
                .chat_validated(client.as_ref(), &request, &params.mail_id)
                .await?;
            input_tokens = input_tokens.saturating_add(response.usage.prompt_tokens);
            output_tokens = output_tokens.saturating_add(response.usage.completion_tokens);
            latency_ms = latency_ms.saturating_add(response.latency_ms);
            if !response.model_echo.is_empty() {
                model_echo = response.model_echo;
            }

            // Merge (T070 §3 step 3): risk union deduplicated by excerpt,
            // clauses from the last segment, advice in first-seen order.
            let (items, clauses, advice) = output.into_parts();
            for item in items {
                if seen_excerpts.insert(item.original_text.clone()) {
                    risk_list.push(item);
                }
            }
            key_clauses = clauses;
            for entry in advice {
                if !compliance_advice.contains(&entry) {
                    compliance_advice.push(entry);
                }
            }
        }
        compliance_advice.truncate(COMPLIANCE_ADVICE_MAX);

        // 6) Build the verdict and persist audit + risk rows atomically.
        let overall_level = derive_overall_level(&risk_list);
        let result = LegalAnalysisResult {
            decision_id: new_uuid(),
            mail_id: params.mail_id.clone(),
            account_id,
            risk_list,
            key_clauses,
            compliance_advice,
            overall_level,
            ai_model: model_echo,
            knowledge_refs: ctx.knowledge_refs.clone(),
            created_at: now_unix(),
        };
        self.persist(&result, input_tokens, output_tokens, latency_ms.max(1))
            .await?;

        // Identifiers, counts, and levels only — never content (dev/09 §5).
        tracing::info!(
            event = "legal_analysis_complete",
            mail_id = %result.mail_id,
            account_id = %result.account_id,
            decision_id = %result.decision_id,
            risks = result.risk_list.len(),
            overall = result.overall_level.as_wire(),
            segments = segment_count,
            knowledge_refs = result.knowledge_refs.len(),
            latency_ms = latency_ms,
            "legal D1 analysis complete"
        );
        Ok(result)
    }

    /// The freshest D1 audit row for this mail within the cache TTL, replayed
    /// from the result JSON stored in `action_description`. Rows that fail to
    /// parse (foreign writers, older formats) fall through to a fresh run.
    async fn cached_result(&self, mail_id: &str) -> AppResult<Option<LegalAnalysisResult>> {
        let row = sqlx::query(
            "SELECT action_description FROM ai_decisions \
             WHERE mail_id = ? AND decision_type LIKE 'risk_alert%' AND created_at > ? \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(mail_id)
        .bind(now_unix() - CACHE_TTL_SECS)
        .fetch_optional(self.state.storage.db().pool())
        .await
        .map_err(map_sqlx_err)?;
        Ok(row.and_then(|r| {
            serde_json::from_str::<LegalAnalysisResult>(&r.get::<String, _>("action_description"))
                .ok()
        }))
    }

    /// `chat()` plus strict D1 validation with exactly one re-prompt retry
    /// (F_D1 §6). Provider transport errors propagate immediately — only an
    /// unparseable/non-conforming body earns the retry.
    async fn chat_validated(
        &self,
        client: &dyn AiProviderClient,
        request: &ChatRequest,
        mail_id: &str,
    ) -> AppResult<(ModelOutput, ChatResponse)> {
        let first = client.chat(request.clone()).await?;
        if let Some(output) = parse_d1_output(&first.text) {
            return Ok((output, first));
        }
        // Identifiers only: the invalid payload itself is never logged.
        tracing::warn!(
            event = "legal_analysis_invalid_output",
            mail_id = %mail_id,
            attempt = 1,
            "D1 output failed schema validation; retrying once"
        );
        let mut retry = request.clone();
        retry.request_id = Uuid::new_v4();
        let second = client.chat(retry).await?;
        match parse_d1_output(&second.text) {
            Some(output) => Ok((output, second)),
            None => {
                tracing::warn!(
                    event = "legal_analysis_invalid_output",
                    mail_id = %mail_id,
                    attempt = 2,
                    "D1 output invalid after retry; degrading"
                );
                Err(AppError::Internal(anyhow::anyhow!(
                    "legal analysis output invalid after retry"
                )))
            }
        }
    }

    /// `from_email` per grounding mail, for the `[Prior Mail: …]` lines.
    async fn chunk_senders(&self, ctx: &RoleContext) -> AppResult<HashMap<String, String>> {
        if ctx.knowledge_refs.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = vec!["?"; ctx.knowledge_refs.len()].join(",");
        let sql = format!("SELECT id, from_email FROM mails WHERE id IN ({placeholders})");
        let mut query = sqlx::query(&sql);
        for mail_id in &ctx.knowledge_refs {
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

    /// One transaction: the append-only `ai_decisions` audit row plus one
    /// `risk_events` row per high/medium risk item (T070 §3 steps 7–8).
    async fn persist(
        &self,
        result: &LegalAnalysisResult,
        input_tokens: u32,
        output_tokens: u32,
        latency_ms: u32,
    ) -> AppResult<()> {
        // The full result JSON backs the 24-hour cache (T070 §3 step 1).
        let payload = serde_json::to_string(result)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize analysis: {e}")))?;
        let knowledge_refs_json = serde_json::to_string(&result.knowledge_refs)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize knowledge refs: {e}")))?;

        let mut tx = self
            .state
            .storage
            .db()
            .pool()
            .begin()
            .await
            .map_err(map_sqlx_err)?;

        sqlx::query(
            "INSERT INTO ai_decisions (id, account_id, mail_id, decision_type, impact, \
                 action_description, knowledge_refs, knowledge_summary, result_description, \
                 ai_model, input_tokens, output_tokens, latency_ms, created_at) \
             VALUES (?, ?, ?, ?, 'risk', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&result.decision_id)
        .bind(&result.account_id)
        .bind(&result.mail_id)
        .bind(decision_type_for(result.overall_level))
        .bind(&payload)
        .bind(&knowledge_refs_json)
        .bind(format!(
            "Grounded on {} prior mails",
            result.knowledge_refs.len()
        ))
        // Content-free summary (dev/09 §5): counts and levels only.
        .bind(format!(
            "Legal D1 analysis: {} risks, overall={}",
            result.risk_list.len(),
            result.overall_level.as_wire()
        ))
        .bind(&result.ai_model)
        .bind(input_tokens as i64)
        .bind(output_tokens as i64)
        .bind(latency_ms as i64)
        .bind(result.created_at)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_err)?;

        for item in &result.risk_list {
            let (risk_level, expires_at): (i64, Option<i64>) = match item.level {
                // High ⇢ T4: never expires (T070 §6, dev/01 risk_events).
                LegalRiskLevel::High => (4, None),
                LegalRiskLevel::Medium => (3, Some(result.created_at + MEDIUM_RISK_TTL_SECS)),
                // Low items never create risk events (T070 §6).
                LegalRiskLevel::Low => continue,
            };
            // Evidence carries the excerpt's hash, never the excerpt (09 §5).
            let evidence = serde_json::json!({
                "d1_finding": item.finding,
                "original_text_hash": original_text_hash(&item.original_text),
            })
            .to_string();
            sqlx::query(
                "INSERT INTO risk_events (id, mail_id, account_id, risk_level, risk_type, \
                     evidence, description, status, expires_at, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?, ?, ?)",
            )
            .bind(new_uuid())
            .bind(&result.mail_id)
            .bind(&result.account_id)
            .bind(risk_level)
            .bind(map_risk_type(item.risk_type))
            .bind(&evidence)
            .bind(&item.finding)
            .bind(expires_at)
            .bind(result.created_at)
            .bind(result.created_at)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_err)?;
        }

        tx.commit().await.map_err(map_sqlx_err)?;
        Ok(())
    }
}

// ── Model-output parsing (D1 §4.4 wire shape: snake_case keys) ───────────────

/// Raw model output. Field names follow the D1 schema exactly (snake_case);
/// all three top-level fields are required, so a missing one fails validation.
/// The shared `LegalRiskLevel`/`LegalRiskType` enums make unknown `level` /
/// `type` tags a hard parse error (strict per T070 §3 step 6).
///
/// Deliberately no `Debug` derive: these structs carry mail excerpts, and the
/// logging red-line (dev/09 §5) is easier to uphold when they cannot be
/// formatted into a log line at all.
#[derive(serde::Deserialize)]
struct ModelOutput {
    risk_list: Vec<ModelRiskItem>,
    key_clauses: ModelKeyClauses,
    compliance_advice: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ModelRiskItem {
    level: LegalRiskLevel,
    #[serde(rename = "type")]
    risk_type: LegalRiskType,
    original_text: String,
    finding: String,
    suggestion: String,
}

#[derive(serde::Deserialize)]
struct ModelKeyClauses {
    payment: Option<String>,
    delivery: Option<String>,
    liability: Option<String>,
    confidentiality: Option<String>,
    dispute_resolution: Option<String>,
}

impl ModelOutput {
    /// Convert into wire shapes, enforcing the D1 length caps defensively.
    fn into_parts(self) -> (Vec<LegalRiskItem>, LegalKeyClauses, Vec<String>) {
        let items = self
            .risk_list
            .into_iter()
            .map(|r| LegalRiskItem {
                level: r.level,
                risk_type: r.risk_type,
                original_text: truncate_chars(&r.original_text, ORIGINAL_TEXT_MAX_CHARS),
                finding: truncate_chars(&r.finding, FINDING_MAX_CHARS),
                suggestion: truncate_chars(&r.suggestion, SUGGESTION_MAX_CHARS),
            })
            .collect();
        let clauses = LegalKeyClauses {
            payment: self.key_clauses.payment,
            delivery: self.key_clauses.delivery,
            liability: self.key_clauses.liability,
            confidentiality: self.key_clauses.confidentiality,
            dispute_resolution: self.key_clauses.dispute_resolution,
        };
        (items, clauses, self.compliance_advice)
    }
}

/// Extract and strictly parse the D1 JSON object from a completion. Tolerates
/// stray prose/fences around the object (first `{` to last `}`), but nothing
/// inside it: missing fields and unknown enum tags fail. Returns `None` on any
/// failure so no fragment of the payload can travel on an error value.
fn parse_d1_output(raw: &str) -> Option<ModelOutput> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str::<ModelOutput>(&raw[start..=end]).ok()
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Worst-of aggregation over the merged risk list (T070 §6).
fn derive_overall_level(risk_list: &[LegalRiskItem]) -> LegalOverallLevel {
    if risk_list.iter().any(|r| r.level == LegalRiskLevel::High) {
        LegalOverallLevel::High
    } else if risk_list.iter().any(|r| r.level == LegalRiskLevel::Medium) {
        LegalOverallLevel::Medium
    } else if !risk_list.is_empty() {
        LegalOverallLevel::Low
    } else {
        LegalOverallLevel::None
    }
}

/// `ai_decisions.decision_type` by overall level (T070 §3 step 7).
fn decision_type_for(level: LegalOverallLevel) -> &'static str {
    match level {
        LegalOverallLevel::High => "risk_alert_t4",
        LegalOverallLevel::Medium => "risk_alert_t3",
        LegalOverallLevel::Low | LegalOverallLevel::None => "risk_alert_t1",
    }
}

/// D1 risk category → `risk_events.risk_type` (T070 §6, dev/01).
fn map_risk_type(risk_type: LegalRiskType) -> &'static str {
    match risk_type {
        LegalRiskType::Payment => "payment_anomaly",
        LegalRiskType::Liability => "amount_threshold",
        LegalRiskType::Dispute => "rule_conflict",
        LegalRiskType::Delivery | LegalRiskType::Confidentiality | LegalRiskType::Other => {
            "context_missing"
        }
    }
}

/// First 8 bytes of `SHA-256(text)` as lowercase hex — the only form of the
/// flagged excerpt ever persisted in `risk_events.evidence` (T070 §6).
fn original_text_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(EVIDENCE_HASH_HEX_CHARS);
    for byte in digest.iter().take(EVIDENCE_HASH_HEX_CHARS / 2) {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Grounding block prepended to every segment's user message (dev/06 §5 order
/// inside the user turn: parties, thread, GTE context; the current mail body
/// follows the block).
fn build_grounding(
    ctx: &RoleContext,
    chunk_senders: &HashMap<String, String>,
    from_name: Option<&str>,
    recipient_domain: Option<&str>,
) -> String {
    let mut block = String::new();
    let from = match from_name {
        Some(name) if !name.trim().is_empty() => {
            format!("{} <{}>", name.trim(), ctx.target_mail.from_email)
        }
        _ => format!("<{}>", ctx.target_mail.from_email),
    };
    block.push_str(&format!("[Parties: from {from}"));
    if let Some(domain) = recipient_domain {
        block.push_str(&format!("; recipient domain {domain}"));
    }
    block.push_str("]\n");
    for mail in &ctx.thread_mails {
        block.push_str(&format!(
            "[Thread Mail: {} from {}: {}]\n",
            format_date(mail.date_sent),
            mail.from_email,
            mail.body
        ));
    }
    for chunk in &ctx.chunks {
        let sender = chunk_senders
            .get(&chunk.mail_id)
            .map(String::as_str)
            .unwrap_or("unknown");
        block.push_str(&format!(
            "[Prior Mail: {} from {}: {}]\n",
            format_date(chunk.date_sent),
            sender,
            chunk.snippet
        ));
    }
    block
}

/// Domain of the first recipient in the `to_addrs` JSON array (T070 §3 step 2).
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

/// Split an oversize body into ≤ `max_bytes` segments on paragraph boundaries
/// (F_D1 §6). A single paragraph longer than the limit is hard-split on char
/// boundaries so the function can never produce an oversize segment or panic
/// on multi-byte text.
fn split_segments(body: &str, max_bytes: usize) -> Vec<String> {
    if body.len() <= max_bytes {
        return vec![body.to_string()];
    }
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    for paragraph in body.split("\n\n") {
        for piece in hard_split(paragraph, max_bytes) {
            let separator = if current.is_empty() { 0 } else { 2 };
            if !current.is_empty() && current.len() + separator + piece.len() > max_bytes {
                segments.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(&piece);
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// Char-boundary-safe hard split for one paragraph exceeding `max_bytes`.
fn hard_split(paragraph: &str, max_bytes: usize) -> Vec<String> {
    if paragraph.len() <= max_bytes {
        return vec![paragraph.to_string()];
    }
    let mut pieces = Vec::new();
    let mut current = String::new();
    for ch in paragraph.chars() {
        if current.len() + ch.len_utf8() > max_bytes {
            pieces.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
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
    use std::sync::Arc;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::ProviderError;
    use crate::ai::types::{FinishReason, TokenUsage};
    use crate::error::IpcError;
    use crate::types::{AiProvider, ErrorCode};
    use crate::util::now_unix;
    use crate::vector::VectorRow;

    // ── Seeding helpers ──────────────────────────────────────────────────────

    /// Account with the legal role + an `account_ai_settings` row routed to
    /// the (mock) OpenAI provider.
    async fn seed_account(state: &AppState, id: &str) {
        let pool = state.storage.db().pool();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 role_type, role_description, created_at, updated_at) \
             VALUES (?, ?, 'Legal Desk', 'imap', 'terra', 'L', 'legal', \
                 'Review inbound contracts for risk.', 0, 0)",
        )
        .bind(id)
        .bind(format!("{id}@corp.com"))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_ai_settings (account_id, auth_level, ai_provider, ai_model, \
                 daily_query_limit, updated_at) \
             VALUES (?, 1, 'openai', 'gpt-test', 50, 0)",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_mail(state: &AppState, id: &str, acc: &str, body: &str, date_sent: i64) {
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_name, from_email, \
                 to_addrs, date_sent, date_received, body_text, snippet, embedding_status, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, 'Contract renewal terms', 'Dana Webb', 'counsel@partner.com', \
                 '[{\"name\":\"Legal Desk\",\"email\":\"legal@corp.com\"}]', ?, ?, ?, ?, \
                 'indexed', 0, 0)",
        )
        .bind(id)
        .bind(acc)
        .bind(format!("<{id}@x>"))
        .bind(date_sent)
        .bind(date_sent)
        .bind(body)
        .bind(truncate_chars(body, 200))
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    /// Embed `text` and upsert it as the mail's chunk in the vector store, so
    /// GTE retrieval has something to ground on (mirrors context.rs tests).
    async fn index_mail(state: &AppState, id: &str, acc: &str, text: &str) {
        let rows = vec![VectorRow {
            chunk_id: format!("{id}:0"),
            mail_id: id.into(),
            chunk_index: 0,
            account_id: acc.into(),
            from_email: "counsel@partner.com".into(),
            date_sent: now_unix(),
            subject: text.into(),
            snippet: text.into(),
            embedding_model: "bge-m3".into(),
            vector: state.embedder.embed(text).unwrap(),
        }];
        state.storage.vectors().upsert(&rows).unwrap();
    }

    const TRIGGER_BODY: &str =
        "the quarterly licensing contract renewal terms and the indemnity clause review";

    /// Account + trigger mail + two semantically related, indexed prior mails
    /// so `knowledge_refs` is non-empty.
    async fn seed_corpus(state: &AppState, acc: &str) {
        seed_account(state, acc).await;
        seed_mail(state, "trigger", acc, TRIGGER_BODY, now_unix()).await;
        index_mail(state, "trigger", acc, TRIGGER_BODY).await;
        let related1 = "prior contract renewal discussed the licensing terms and indemnity";
        let related2 = "the indemnity clause review from last quarter licensing contract";
        seed_mail(state, "k1", acc, related1, now_unix() - 100).await;
        index_mail(state, "k1", acc, related1).await;
        seed_mail(state, "k2", acc, related2, now_unix() - 200).await;
        index_mail(state, "k2", acc, related2).await;
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

    const HIGH_EXCERPT: &str = "Payment due within 90 days of invoice";

    /// A schema-conforming D1 verdict: one high, one medium, one low risk.
    fn d1_json() -> String {
        serde_json::json!({
            "risk_list": [
                {
                    "level": "high",
                    "type": "payment",
                    "original_text": HIGH_EXCERPT,
                    "finding": "Unusually long payment term",
                    "suggestion": "Negotiate net-30 payment terms"
                },
                {
                    "level": "medium",
                    "type": "liability",
                    "original_text": "Liability is unlimited for both parties",
                    "finding": "Unlimited liability exposure",
                    "suggestion": "Cap liability at the contract value"
                },
                {
                    "level": "low",
                    "type": "other",
                    "original_text": "Notices may be sent by email",
                    "finding": "Email-only notice clause",
                    "suggestion": "Add a registered-mail fallback"
                }
            ],
            "key_clauses": {
                "payment": "Net 90 from invoice date",
                "liability": "Unlimited, both parties",
                "dispute_resolution": "Arbitration in Singapore"
            },
            "compliance_advice": [
                "Cap liability at 100% of contract value",
                "Shorten payment terms to net 30"
            ]
        })
        .to_string()
    }

    async fn analyze(
        state: &AppState,
        mail_id: &str,
        force_new: bool,
    ) -> AppResult<LegalAnalysisResult> {
        LegalAnalysisPipeline::new(state)
            .run(&AnalyzeLegalRiskParams {
                mail_id: mail_id.into(),
                force_new,
            })
            .await
    }

    // ── Pipeline integration tests ───────────────────────────────────────────

    #[tokio::test]
    async fn success_returns_result_and_writes_audit_and_risk_rows() {
        let (state, _rx) = AppState::test_state().await;
        seed_corpus(&state, "acct").await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response(d1_json())));

        let result = analyze(&state, "trigger", true).await.unwrap();
        assert_eq!(result.mail_id, "trigger");
        assert_eq!(result.account_id, "acct");
        assert_eq!(result.risk_list.len(), 3);
        assert_eq!(result.overall_level, LegalOverallLevel::High);
        assert!(!result.knowledge_refs.is_empty(), "grounded on prior mails");
        assert_eq!(result.ai_model, "gpt-test-echo");
        assert_eq!(result.compliance_advice.len(), 2);
        assert_eq!(
            result.key_clauses.dispute_resolution.as_deref(),
            Some("Arbitration in Singapore")
        );

        // Audit row (dev/06 §9, T070 §3 step 7).
        let pool = state.storage.db().pool();
        let row = sqlx::query(
            "SELECT decision_type, impact, knowledge_refs, ai_model, input_tokens, \
                 output_tokens, latency_ms FROM ai_decisions WHERE id = ?",
        )
        .bind(&result.decision_id)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("decision_type"), "risk_alert_t4");
        assert_eq!(row.get::<String, _>("impact"), "risk");
        let refs: Vec<String> =
            serde_json::from_str(&row.get::<String, _>("knowledge_refs")).unwrap();
        assert!(!refs.is_empty(), "knowledge_refs is a non-empty JSON array");
        assert_eq!(refs, result.knowledge_refs);
        assert!(row.get::<i64, _>("latency_ms") > 0);
        assert!(row.get::<i64, _>("input_tokens") > 0);
        assert!(row.get::<i64, _>("output_tokens") > 0);

        // Risk rows: high → 4/never expires, medium → 3/expires, low → none.
        let events = sqlx::query(
            "SELECT risk_level, risk_type, status, expires_at FROM risk_events \
             WHERE mail_id = 'trigger' ORDER BY risk_level DESC",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(events.len(), 2, "low items never create risk events");
        assert_eq!(events[0].get::<i64, _>("risk_level"), 4);
        assert_eq!(events[0].get::<String, _>("risk_type"), "payment_anomaly");
        assert_eq!(events[0].get::<String, _>("status"), "open");
        assert_eq!(events[0].get::<Option<i64>, _>("expires_at"), None);
        assert_eq!(events[1].get::<i64, _>("risk_level"), 3);
        assert_eq!(events[1].get::<String, _>("risk_type"), "amount_threshold");
        let medium_expiry = events[1].get::<Option<i64>, _>("expires_at").unwrap();
        assert!(medium_expiry > now_unix(), "medium events expire in 7 days");
    }

    #[tokio::test]
    async fn cache_hit_skips_provider_and_force_new_bypasses_cache() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct").await;
        seed_mail(
            &state,
            "m1",
            "acct",
            "renewal clause discussion",
            now_unix(),
        )
        .await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response(d1_json())));

        let first = analyze(&state, "m1", false).await.unwrap();
        assert_eq!(mock.chat_call_count(), 1);

        // Within 24h, force_new = false replays the stored verdict.
        let cached = analyze(&state, "m1", false).await.unwrap();
        assert_eq!(mock.chat_call_count(), 1, "no provider call on cache hit");
        assert_eq!(cached, first);

        // force_new = true ignores the cache and produces a new decision row.
        mock.push_chat(Ok(response(d1_json())));
        let fresh = analyze(&state, "m1", true).await.unwrap();
        assert_eq!(mock.chat_call_count(), 2);
        assert_ne!(fresh.decision_id, first.decision_id);
    }

    #[tokio::test]
    async fn invalid_json_retries_once_then_succeeds() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct").await;
        seed_mail(&state, "m1", "acct", "contract body", now_unix()).await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response("the contract looks risky overall")));
        mock.push_chat(Ok(response(d1_json())));

        let result = analyze(&state, "m1", true).await.unwrap();
        assert_eq!(mock.chat_call_count(), 2, "exactly one retry");
        assert_eq!(result.overall_level, LegalOverallLevel::High);
    }

    #[tokio::test]
    async fn invalid_json_twice_returns_internal_and_persists_nothing() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct").await;
        seed_mail(&state, "m1", "acct", "contract body", now_unix()).await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response("no json at all")));
        mock.push_chat(Ok(response("{\"risk_list\": \"not an array\"}")));

        let err = analyze(&state, "m1", true).await.unwrap_err();
        let ipc: IpcError = err.into();
        assert_eq!(ipc.code, ErrorCode::Internal);
        assert_eq!(mock.chat_call_count(), 2);

        let pool = state.storage.db().pool();
        let (decisions,): (i64,) = sqlx::query_as("SELECT count(*) FROM ai_decisions")
            .fetch_one(pool)
            .await
            .unwrap();
        let (risks,): (i64,) = sqlx::query_as("SELECT count(*) FROM risk_events")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!((decisions, risks), (0, 0), "failed runs leave no rows");
    }

    #[tokio::test]
    async fn unknown_risk_level_is_rejected() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct").await;
        seed_mail(&state, "m1", "acct", "contract body", now_unix()).await;
        let mock = register_mock(&state);
        let bad = serde_json::json!({
            "risk_list": [{
                "level": "critical",
                "type": "payment",
                "original_text": "x",
                "finding": "y",
                "suggestion": "z"
            }],
            "key_clauses": {},
            "compliance_advice": []
        })
        .to_string();
        mock.push_chat(Ok(response(bad.clone())));
        mock.push_chat(Ok(response(bad)));

        let err = analyze(&state, "m1", true).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(mock.chat_call_count(), 2, "unknown level burns the retry");
    }

    #[tokio::test]
    async fn provider_unreachable_propagates_without_retry() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct").await;
        seed_mail(&state, "m1", "acct", "contract body", now_unix()).await;
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
        seed_account(&state, "acct").await;
        register_mock(&state);
        let err = analyze(&state, "missing", true).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn oversize_body_segments_and_merges() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct").await;
        // Two paragraphs of ~56K bytes each → > 80K total → two segments.
        let body = format!(
            "{}\n\n{}",
            "clause ".repeat(8_000),
            "payment ".repeat(7_000)
        );
        assert!(body.len() > SEGMENT_CHAR_LIMIT);
        seed_mail(&state, "big", "acct", &body, now_unix()).await;
        let mock = register_mock(&state);

        let seg1 = serde_json::json!({
            "risk_list": [{
                "level": "high",
                "type": "payment",
                "original_text": HIGH_EXCERPT,
                "finding": "Unusually long payment term",
                "suggestion": "Negotiate net-30 payment terms"
            }],
            "key_clauses": { "payment": "Net 90 from invoice date" },
            "compliance_advice": ["Cap liability", "Shorten terms"]
        })
        .to_string();
        let seg2 = serde_json::json!({
            "risk_list": [
                {
                    "level": "high",
                    "type": "payment",
                    "original_text": HIGH_EXCERPT,
                    "finding": "Unusually long payment term",
                    "suggestion": "Negotiate net-30 payment terms"
                },
                {
                    "level": "medium",
                    "type": "dispute",
                    "original_text": "Disputes resolved in seller's home court",
                    "finding": "One-sided forum selection",
                    "suggestion": "Propose neutral arbitration"
                }
            ],
            "key_clauses": { "payment": "Net 30 from invoice date" },
            "compliance_advice": ["Add arbitration clause", "Cap liability"]
        })
        .to_string();
        mock.push_chat(Ok(response(seg1)));
        mock.push_chat(Ok(response(seg2)));

        let result = analyze(&state, "big", true).await.unwrap();
        assert_eq!(mock.chat_call_count(), 2, "one provider call per segment");
        // Union deduplicated by original_text.
        assert_eq!(result.risk_list.len(), 2);
        // key_clauses come from the last segment.
        assert_eq!(
            result.key_clauses.payment.as_deref(),
            Some("Net 30 from invoice date")
        );
        // First three distinct advice entries, in first-seen order.
        assert_eq!(
            result.compliance_advice,
            vec!["Cap liability", "Shorten terms", "Add arbitration clause"]
        );
        assert_eq!(result.overall_level, LegalOverallLevel::High);

        // Both merged risks landed in risk_events (high + medium).
        let (risk_rows,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM risk_events WHERE mail_id = 'big'")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(risk_rows, 2);
    }

    /// 09 §5 / T070 §6: the persisted risk evidence and the audit summary
    /// carry no excerpt and no body text — only the SHA-256 prefix.
    #[tokio::test]
    async fn evidence_stores_hash_prefix_never_the_excerpt() {
        let (state, _rx) = AppState::test_state().await;
        seed_account(&state, "acct").await;
        seed_mail(&state, "m1", "acct", TRIGGER_BODY, now_unix()).await;
        let mock = register_mock(&state);
        mock.push_chat(Ok(response(d1_json())));

        analyze(&state, "m1", true).await.unwrap();

        let pool = state.storage.db().pool();
        let evidence: String =
            sqlx::query("SELECT evidence FROM risk_events WHERE mail_id = 'm1' AND risk_level = 4")
                .fetch_one(pool)
                .await
                .unwrap()
                .get("evidence");
        assert!(!evidence.contains(HIGH_EXCERPT), "no excerpt in evidence");
        let parsed: serde_json::Value = serde_json::from_str(&evidence).unwrap();
        let expected = {
            let digest = Sha256::digest(HIGH_EXCERPT.as_bytes());
            digest
                .iter()
                .take(8)
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert_eq!(
            parsed.get("original_text_hash").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );
        assert_eq!(
            parsed.get("d1_finding").and_then(|v| v.as_str()),
            Some("Unusually long payment term")
        );

        // The content-free audit summary never carries body or excerpt text.
        let summary: String =
            sqlx::query("SELECT result_description FROM ai_decisions WHERE mail_id = 'm1'")
                .fetch_one(pool)
                .await
                .unwrap()
                .get("result_description");
        assert_eq!(summary, "Legal D1 analysis: 3 risks, overall=high");
        assert!(!summary.contains(TRIGGER_BODY));
        assert!(!summary.contains(HIGH_EXCERPT));
    }

    // ── Pure-helper unit tests ───────────────────────────────────────────────

    fn item(level: LegalRiskLevel) -> LegalRiskItem {
        LegalRiskItem {
            level,
            risk_type: LegalRiskType::Other,
            original_text: "x".into(),
            finding: "y".into(),
            suggestion: "z".into(),
        }
    }

    #[test]
    fn overall_level_is_worst_of_list() {
        assert_eq!(derive_overall_level(&[]), LegalOverallLevel::None);
        assert_eq!(
            derive_overall_level(&[item(LegalRiskLevel::Low)]),
            LegalOverallLevel::Low
        );
        assert_eq!(
            derive_overall_level(&[item(LegalRiskLevel::Low), item(LegalRiskLevel::Medium)]),
            LegalOverallLevel::Medium
        );
        assert_eq!(
            derive_overall_level(&[item(LegalRiskLevel::Medium), item(LegalRiskLevel::High)]),
            LegalOverallLevel::High
        );
    }

    #[test]
    fn decision_type_maps_levels_to_t4_t3_t1() {
        assert_eq!(decision_type_for(LegalOverallLevel::High), "risk_alert_t4");
        assert_eq!(
            decision_type_for(LegalOverallLevel::Medium),
            "risk_alert_t3"
        );
        assert_eq!(decision_type_for(LegalOverallLevel::Low), "risk_alert_t1");
        assert_eq!(decision_type_for(LegalOverallLevel::None), "risk_alert_t1");
    }

    #[test]
    fn risk_type_mapping_matches_card() {
        assert_eq!(map_risk_type(LegalRiskType::Payment), "payment_anomaly");
        assert_eq!(map_risk_type(LegalRiskType::Liability), "amount_threshold");
        assert_eq!(map_risk_type(LegalRiskType::Dispute), "rule_conflict");
        assert_eq!(map_risk_type(LegalRiskType::Delivery), "context_missing");
        assert_eq!(
            map_risk_type(LegalRiskType::Confidentiality),
            "context_missing"
        );
        assert_eq!(map_risk_type(LegalRiskType::Other), "context_missing");
    }

    #[test]
    fn parse_tolerates_fences_but_rejects_bad_shapes() {
        let wrapped = format!("```json\n{}\n```", d1_json());
        assert!(parse_d1_output(&wrapped).is_some());
        assert!(parse_d1_output(&d1_json()).is_some());
        assert!(parse_d1_output("plain prose, no object").is_none());
        // Missing required top-level field.
        assert!(parse_d1_output(r#"{"risk_list":[],"key_clauses":{}}"#).is_none());
        // Unknown risk type tag.
        let bad_type = r#"{"risk_list":[{"level":"high","type":"weather","original_text":"a","finding":"b","suggestion":"c"}],"key_clauses":{},"compliance_advice":[]}"#;
        assert!(parse_d1_output(bad_type).is_none());
    }

    #[test]
    fn parse_caps_field_lengths_defensively() {
        let long = "x".repeat(500);
        let raw = format!(
            r#"{{"risk_list":[{{"level":"low","type":"other","original_text":"{long}","finding":"{long}","suggestion":"{long}"}}],"key_clauses":{{}},"compliance_advice":[]}}"#
        );
        let (items, _, _) = parse_d1_output(&raw).unwrap().into_parts();
        assert_eq!(items[0].original_text.chars().count(), 120);
        assert_eq!(items[0].finding.chars().count(), 80);
        assert_eq!(items[0].suggestion.chars().count(), 80);
    }

    #[test]
    fn split_segments_respects_limit_and_paragraphs() {
        // Under the limit: one segment, untouched.
        assert_eq!(split_segments("short body", 100), vec!["short body"]);
        // Paragraph-boundary split.
        let body = format!("{}\n\n{}", "a".repeat(60), "b".repeat(60));
        let segments = split_segments(&body, 100);
        assert_eq!(segments.len(), 2);
        assert!(segments.iter().all(|s| s.len() <= 100));
        // A single oversize paragraph hard-splits without panicking on
        // multi-byte chars.
        let cjk = "法".repeat(50); // 150 bytes
        let segments = split_segments(&cjk, 100);
        assert!(segments.len() >= 2);
        assert!(segments.iter().all(|s| s.len() <= 100));
        assert_eq!(segments.concat(), cjk);
    }

    #[test]
    fn excerpt_hash_is_first_eight_sha256_bytes_hex() {
        // echo -n "abc" | sha256sum → ba7816bf8f01cfea...
        assert_eq!(original_text_hash("abc"), "ba7816bf8f01cfea");
        assert_eq!(original_text_hash("abc").len(), EVIDENCE_HASH_HEX_CHARS);
    }

    #[test]
    fn first_recipient_domain_parses_to_addrs_json() {
        assert_eq!(
            first_recipient_domain(r#"[{"name":"Legal","email":"legal@Corp.COM"}]"#).as_deref(),
            Some("corp.com")
        );
        assert_eq!(first_recipient_domain("[]"), None);
        assert_eq!(first_recipient_domain("not json"), None);
        assert_eq!(
            first_recipient_domain(r#"[{"name":"x","email":"no-at-sign"}]"#),
            None
        );
    }

    #[test]
    fn system_prompt_embeds_role_and_schema() {
        let prompt = legal_system_prompt("You are the legal assistant for legal@corp.com.");
        assert!(prompt.contains("senior corporate legal counsel"));
        assert!(prompt.contains("legal@corp.com"));
        assert!(prompt.contains(LEGAL_JSON_SCHEMA));
        assert!(prompt.contains("Output ONLY the JSON object"));
    }
}
