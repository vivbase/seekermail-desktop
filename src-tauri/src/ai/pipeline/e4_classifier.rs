//! E4 sensitive-mail pre-scan classifier (T084 §3, F_E4 §4, AI_MODES §5).
//!
//! Two phases:
//!
//! 1. **Rule chain** (fast path, no LLM): spam markers route to Trash;
//!    three *non-disableable* hard rules route to the forced-draft path —
//!    document attachments, monetary amounts over the threshold, and senders
//!    flagged as important contacts (`contacts.is_trusted = 1`; the schema has
//!    no dedicated "important" column, so `is_trusted` carries that semantic —
//!    aligned with F_E4 §4.3). User-defined rules from
//!    `app_settings['ai.sensitive_rules']` are evaluated after the hard rules.
//! 2. **LLM binary classifier** when no rule fires: 200 ms timeout, one-word
//!    answer; timeout or an unavailable provider degrades to `Normal`
//!    (F_E4 §6 — the low-risk degradation), keeping P95 ≤ 500 ms.
//!
//! Currency units that are CJK in the spec are matched through `\u{..}`
//! escapes — no raw CJK in source (repo language rule).

use std::time::Duration;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

use crate::ai::provider::AiProviderClient;
use crate::ai::types::{Capability, ChatMessage, ChatRequest, ChatRole};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::{map_sqlx_err, SettingRepo};
use crate::util::truncate_chars;

use super::PipelineMail;

/// `app_settings` key holding the user-defined sensitive rules (JSON array).
pub const SENSITIVE_RULES_KEY: &str = "ai.sensitive_rules";

/// LLM classification timeout — keeps the E4 path inside its P95 ≤ 500 ms
/// budget (T084 §6).
pub const E4_LLM_TIMEOUT_MS: u64 = 200;

/// Default monetary thresholds (T084 §6): the CJK-unit bucket and the
/// western-currency bucket. Rough magnitude gates, no FX conversion.
pub const AMOUNT_THRESHOLD_CNY: f64 = 10_000.0;
pub const AMOUNT_THRESHOLD_USD: f64 = 1_000.0;

/// Currency/amount detector; CJK units via `\u{..}` escapes (T084 §6).
/// `\u{00A5}`/`\u{FFE5}` = yen/yuan signs, `\u{20AC}` = euro, `\u{00A3}` =
/// pound, `\u{5143}` = yuan unit, `\u{4E07}` = ten-thousand unit.
static AMOUNT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"[$\u{20AC}\u{00A5}\u{00A3}\u{FFE5}]\s*[\d,]{3,}|[\d,]{3,}\s*(\u{5143}|\u{4E07}|USD|CNY|EUR)",
    )
    .expect("amount regex is valid")
});

/// Automated-sender markers for the spam rule (subset of the E2 list).
static SPAM_SENDER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(no-?reply|mailer-daemon|bounce)\b").expect("spam sender regex is valid")
});

/// Bulk-mail subject markers (CJK advertisement tag via escapes).
static SPAM_SUBJECT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)unsubscribe|newsletter|auto-?reply|out of office|\[\u{5E7F}\u{544A}\]")
        .expect("spam subject regex is valid")
});

/// Document attachment extensions that force the Pending draft path
/// (hard rule, AI_MODES §5.5).
const DOCUMENT_EXTENSIONS: [&str; 4] = [".pdf", ".docx", ".xlsx", ".doc"];

const E4_SYSTEM: &str = "Classify this email as 'sensitive' or 'normal'. Reply with one word only.";
/// Body prefix fed to the LLM (subject + opening text, never the full mail).
const E4_BODY_CHARS: usize = 500;

/// Spam-score floor for the automated-sender spam rule (F_E4 §4.1).
const SPAM_SCORE_FLOOR: f64 = 0.7;

/// Classification outcome (T084 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum E4Outcome {
    Spam,
    Sensitive { reason: String, risk_type: String },
    Normal,
}

/// One user-defined sensitive rule from `app_settings['ai.sensitive_rules']`:
/// `{"type": "sender" | "keyword", "value": "..."}`. Unknown shapes are
/// skipped silently (forward compatibility; the config UI lands post-v0.7).
#[derive(Debug, Clone, Deserialize)]
pub struct SensitiveRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    pub value: String,
}

/// DB-derived context for one classification — gathered by [`load_context`]
/// so [`classify`] stays unit-testable without a database.
#[derive(Debug, Clone, Default)]
pub struct E4Context {
    /// Non-inline attachment filenames for the mail.
    pub attachment_names: Vec<String>,
    /// `contacts.interaction_count` for the sender (`0` = first contact).
    pub sender_interaction_count: i64,
    /// `contacts.is_trusted = 1` — the "important contact" hard rule.
    pub sender_is_trusted: bool,
    /// Parsed user-defined rules.
    pub custom_rules: Vec<SensitiveRule>,
    /// The account's resolved model name (for the LLM request).
    pub model: String,
}

/// Gather the classifier's DB context for one mail.
pub async fn load_context(state: &AppState, mail: &PipelineMail) -> AppResult<E4Context> {
    let pool = state.storage.db().pool();
    let attachment_names: Vec<(String,)> =
        sqlx::query_as("SELECT filename FROM attachments WHERE mail_id = ? AND is_inline = 0")
            .bind(&mail.id)
            .fetch_all(pool)
            .await
            .map_err(map_sqlx_err)?;
    let contact: Option<(i64, i64)> =
        sqlx::query_as("SELECT interaction_count, is_trusted FROM contacts WHERE email = ?")
            .bind(mail.from_email.trim().to_lowercase())
            .fetch_optional(pool)
            .await
            .map_err(map_sqlx_err)?;
    let custom_rules = SettingRepo::new(state.storage.db())
        .get(SENSITIVE_RULES_KEY)
        .await?
        .and_then(|raw| serde_json::from_str::<Vec<SensitiveRule>>(&raw).ok())
        .unwrap_or_default();
    let model = state
        .ai
        .account_config(&mail.account_id)
        .await
        .ok()
        .and_then(|cfg| cfg.model)
        .unwrap_or_default();
    Ok(E4Context {
        attachment_names: attachment_names.into_iter().map(|(f,)| f).collect(),
        sender_interaction_count: contact.map(|(c, _)| c).unwrap_or(0),
        sender_is_trusted: contact.map(|(_, t)| t != 0).unwrap_or(false),
        custom_rules,
        model,
    })
}

/// Parse the first matched amount in `text` and compare it against the
/// per-unit threshold. `\u{4E07}` (ten-thousand) multiplies the figure.
fn amount_over_threshold(text: &str) -> bool {
    for m in AMOUNT_RE.find_iter(text) {
        let matched = m.as_str();
        let digits: String = matched
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let Ok(mut value) = digits.parse::<f64>() else {
            continue;
        };
        let is_cjk_unit = matched.contains('\u{00A5}')
            || matched.contains('\u{FFE5}')
            || matched.contains('\u{5143}')
            || matched.contains('\u{4E07}')
            || matched.contains("CNY");
        if matched.contains('\u{4E07}') {
            value *= 10_000.0;
        }
        let threshold = if is_cjk_unit {
            AMOUNT_THRESHOLD_CNY
        } else {
            AMOUNT_THRESHOLD_USD
        };
        if value > threshold {
            return true;
        }
    }
    false
}

/// Whether `text` mentions any monetary amount at all (shared with the T085
/// six-point self-check, which compares draft vs. original).
pub fn mentions_amount(text: &str) -> bool {
    AMOUNT_RE.is_match(text)
}

/// Rule-chain pass (pure). `None` = inconclusive, the LLM decides.
fn rule_chain(mail: &PipelineMail, ctx: &E4Context) -> Option<E4Outcome> {
    // ── Spam rules (F_E4 §4.1) ──────────────────────────────────────────────
    if mail.imap_flags.contains("Junk") {
        return Some(E4Outcome::Spam);
    }
    if SPAM_SENDER_RE.is_match(&mail.from_email)
        && mail.spam_score.unwrap_or(0.0) > SPAM_SCORE_FLOOR
    {
        return Some(E4Outcome::Spam);
    }
    if SPAM_SUBJECT_RE.is_match(&mail.subject) && ctx.sender_interaction_count == 0 {
        return Some(E4Outcome::Spam);
    }

    // ── Hard sensitive rules — non-disableable (AI_MODES §5.5) ──────────────
    if mail.has_attachments != 0 {
        let has_document = ctx.attachment_names.iter().any(|name| {
            let lower = name.to_lowercase();
            DOCUMENT_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        });
        if has_document {
            return Some(E4Outcome::Sensitive {
                reason: "Contains document attachment".into(),
                risk_type: "payment_anomaly".into(),
            });
        }
    }
    if amount_over_threshold(mail.text()) {
        return Some(E4Outcome::Sensitive {
            reason: "Mentions a monetary amount above the threshold".into(),
            risk_type: "payment_anomaly".into(),
        });
    }
    if ctx.sender_is_trusted {
        return Some(E4Outcome::Sensitive {
            reason: "Sender is an important contact".into(),
            risk_type: "identity_unknown".into(),
        });
    }

    // ── User-defined rules ──────────────────────────────────────────────────
    for rule in &ctx.custom_rules {
        let value = rule.value.to_lowercase();
        if value.is_empty() {
            continue;
        }
        let hit = match rule.rule_type.as_str() {
            "sender" => mail.from_email.to_lowercase().contains(&value),
            "keyword" => {
                mail.subject.to_lowercase().contains(&value)
                    || mail.text().to_lowercase().contains(&value)
            }
            _ => false,
        };
        if hit {
            return Some(E4Outcome::Sensitive {
                reason: "Matched a user-defined sensitive rule".into(),
                risk_type: "rule_conflict".into(),
            });
        }
    }
    None
}

/// Full two-phase classification. `client = None` (provider not configured)
/// skips the LLM phase entirely — rules-only, then `Normal` (F_E4 §6).
pub async fn classify(
    mail: &PipelineMail,
    ctx: &E4Context,
    client: Option<&dyn AiProviderClient>,
) -> E4Outcome {
    if let Some(outcome) = rule_chain(mail, ctx) {
        return outcome;
    }

    let Some(client) = client else {
        return E4Outcome::Normal;
    };

    let request = ChatRequest {
        model: ctx.model.clone(),
        system: E4_SYSTEM.into(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: format!(
                "Subject: {}\n\n{}",
                mail.subject,
                truncate_chars(mail.text(), E4_BODY_CHARS)
            ),
        }],
        max_tokens: 20,
        temperature: 0.0,
        stop: Vec::new(),
        purpose: Capability::RiskReason,
        request_id: uuid::Uuid::new_v4(),
    };

    match tokio::time::timeout(
        Duration::from_millis(E4_LLM_TIMEOUT_MS),
        client.chat(request),
    )
    .await
    {
        Ok(Ok(response)) => {
            if response.text.to_lowercase().contains("sensitive") {
                E4Outcome::Sensitive {
                    reason: "AI classifier flagged the mail as sensitive".into(),
                    risk_type: "context_missing".into(),
                }
            } else {
                E4Outcome::Normal
            }
        }
        Ok(Err(e)) => {
            // Provider failure → Normal (F_E4 §6: rules-only when the LLM is
            // unavailable). Error class only — never content.
            tracing::warn!(
                event = "e4_llm_failed",
                mail_id = %mail.id,
                error_class = %e,
                "e4 llm classification failed; treating mail as normal"
            );
            E4Outcome::Normal
        }
        Err(_) => {
            tracing::debug!(
                event = "e4_llm_timeout",
                mail_id = %mail.id,
                timeout_ms = E4_LLM_TIMEOUT_MS,
                "e4 llm classification timed out; treating mail as normal"
            );
            E4Outcome::Normal
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::provider::{ChatDeltaStream, ProviderError};
    use crate::ai::types::{ChatResponse, FinishReason, ProviderHealth, TokenUsage};
    use crate::types::AiProvider;

    fn mail() -> PipelineMail {
        PipelineMail {
            id: "m1".into(),
            account_id: "acc".into(),
            thread_id: None,
            subject: "Renewal terms".into(),
            from_email: "daniel@vendorco.example".into(),
            to_addrs: "[]".into(),
            cc_addrs: "[]".into(),
            body_text: Some("Could you confirm the renewal terms we discussed?".into()),
            snippet: None,
            imap_flags: "[]".into(),
            spam_score: None,
            has_attachments: 0,
            is_sent: 0,
        }
    }

    fn answer(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: text.into(),
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "gpt-4o".into(),
            latency_ms: 50,
        })
    }

    #[tokio::test]
    async fn pdf_attachment_is_sensitive_without_llm() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        let mut m = mail();
        m.has_attachments = 1;
        let ctx = E4Context {
            attachment_names: vec!["Contract-Final.PDF".into()],
            ..Default::default()
        };
        let outcome = classify(&m, &ctx, Some(&mock)).await;
        assert!(matches!(
            outcome,
            E4Outcome::Sensitive { ref risk_type, .. } if risk_type == "payment_anomaly"
        ));
        assert_eq!(mock.chat_call_count(), 0);
    }

    #[tokio::test]
    async fn junk_flag_and_scored_noreply_are_spam() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        let mut junk = mail();
        junk.imap_flags = r#"["\\Junk"]"#.into();
        assert_eq!(
            classify(&junk, &E4Context::default(), Some(&mock)).await,
            E4Outcome::Spam
        );

        let mut noreply = mail();
        noreply.from_email = "noreply@shop.example".into();
        noreply.spam_score = Some(0.9);
        assert_eq!(
            classify(&noreply, &E4Context::default(), Some(&mock)).await,
            E4Outcome::Spam
        );
        assert_eq!(mock.chat_call_count(), 0);
    }

    #[tokio::test]
    async fn newsletter_subject_from_first_contact_is_spam() {
        let mut m = mail();
        m.subject = "Spring Newsletter — Unsubscribe anytime".into();
        let ctx = E4Context {
            sender_interaction_count: 0,
            ..Default::default()
        };
        assert_eq!(classify(&m, &ctx, None).await, E4Outcome::Spam);
        // A known correspondent with the same subject is NOT spam-ruled.
        let known = E4Context {
            sender_interaction_count: 5,
            ..Default::default()
        };
        assert_eq!(classify(&m, &known, None).await, E4Outcome::Normal);
    }

    #[tokio::test]
    async fn large_amount_is_sensitive() {
        let mut m = mail();
        m.body_text = Some("The settlement amount is $4,200,000 due Friday.".into());
        let outcome = classify(&m, &E4Context::default(), None).await;
        assert!(matches!(
            outcome,
            E4Outcome::Sensitive { ref risk_type, .. } if risk_type == "payment_anomaly"
        ));
        // A small figure stays normal.
        let mut small = mail();
        small.body_text = Some("Lunch was $1,2".into());
        assert_eq!(
            classify(&small, &E4Context::default(), None).await,
            E4Outcome::Normal
        );
    }

    #[tokio::test]
    async fn trusted_sender_is_sensitive_without_llm() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        let ctx = E4Context {
            sender_is_trusted: true,
            ..Default::default()
        };
        let outcome = classify(&mail(), &ctx, Some(&mock)).await;
        assert!(matches!(
            outcome,
            E4Outcome::Sensitive { ref risk_type, .. } if risk_type == "identity_unknown"
        ));
        assert_eq!(mock.chat_call_count(), 0);
    }

    #[tokio::test]
    async fn custom_rules_match_sender_and_keyword() {
        let ctx = E4Context {
            custom_rules: vec![SensitiveRule {
                rule_type: "keyword".into(),
                value: "acquisition".into(),
            }],
            ..Default::default()
        };
        let mut m = mail();
        m.body_text = Some("Board update on the planned acquisition.".into());
        assert!(matches!(
            classify(&m, &ctx, None).await,
            E4Outcome::Sensitive { ref risk_type, .. } if risk_type == "rule_conflict"
        ));
    }

    #[tokio::test]
    async fn llm_decides_when_rules_miss() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(answer("sensitive"));
        assert!(matches!(
            classify(&mail(), &E4Context::default(), Some(&mock)).await,
            E4Outcome::Sensitive { .. }
        ));
        mock.push_chat(answer("normal"));
        assert_eq!(
            classify(&mail(), &E4Context::default(), Some(&mock)).await,
            E4Outcome::Normal
        );
    }

    #[tokio::test]
    async fn llm_unavailable_or_absent_is_normal() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::Unreachable("down".into())));
        assert_eq!(
            classify(&mail(), &E4Context::default(), Some(&mock)).await,
            E4Outcome::Normal
        );
        assert_eq!(
            classify(&mail(), &E4Context::default(), None).await,
            E4Outcome::Normal
        );
    }

    /// A provider that never answers inside the 200 ms window.
    struct SlowProvider;

    #[async_trait]
    impl AiProviderClient for SlowProvider {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            tokio::time::sleep(Duration::from_millis(2_000)).await;
            Err(ProviderError::Canceled)
        }
        async fn chat_stream(&self, _req: ChatRequest) -> Result<ChatDeltaStream, ProviderError> {
            Err(ProviderError::Canceled)
        }
        async fn health(&self) -> Result<ProviderHealth, ProviderError> {
            Err(ProviderError::Unreachable("slow".into()))
        }
        fn id(&self) -> AiProvider {
            AiProvider::Openai
        }
        fn context_window(&self) -> usize {
            8_192
        }
    }

    #[tokio::test(start_paused = true)]
    async fn llm_timeout_is_normal() {
        let slow = SlowProvider;
        // Paused-clock auto-advance fires the 200 ms timeout before the 2 s
        // provider sleep completes; no real waiting happens.
        assert_eq!(
            classify(&mail(), &E4Context::default(), Some(&slow)).await,
            E4Outcome::Normal
        );
    }

    #[test]
    fn amount_threshold_logic() {
        assert!(amount_over_threshold("Invoice total $12,500 attached"));
        assert!(!amount_over_threshold("Coffee was $4,50 yesterday"));
        assert!(amount_over_threshold("Total 25,000 CNY for the batch"));
        assert!(!amount_over_threshold("about 5,000 CNY"));
        assert!(!amount_over_threshold("no figures at all"));
        assert!(mentions_amount("price: $1,200"));
        assert!(!mentions_amount("no money here"));
    }
}
