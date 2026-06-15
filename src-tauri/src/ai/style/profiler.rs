//! Two-stage style profiling (T075 §3, F_E5 §4.2/§4.3).
//!
//! Stage 1 sends each group of [`GROUP_SIZE`](super::GROUP_SIZE) samples
//! through one `chat` call and extracts a partial six-dimension summary;
//! Stage 2 merges every partial into the final summary with one more call.
//! Both stages run at temperature 0.2 with `purpose: StyleProfile`, so the F4
//! matrix and the daily-limit guardrail route them like any other AI call.
//!
//! The persisted/wire shape is [`StyleProfileJson`] — serialized with
//! `snake_case` keys to match the stored schema of F_E5 §4.2 exactly (the same
//! JSON lands in `account_ai_settings.style_profile`). `sample_snippets` are
//! kept for local audit only and are never injected into prompts (F_E5 §4.4).
//!
//! Log safety (09 §5): logs carry `account_id`, counts, and latency only.
//! Parse failures report response *length*, never response text.

use serde::{Deserialize, Serialize};
use specta::Type;
use uuid::Uuid;

use crate::ai::provider::AiProviderClient;
use crate::ai::types::{Capability, ChatMessage, ChatRequest, ChatRole};
use crate::error::{AppError, AppResult};
use crate::types::ErrorCode;
use crate::util::{now_unix, truncate_chars};

use super::sampler::{uniform_indices, StyleSample};
use super::{GROUP_SIZE, MIN_SAMPLES, SNIPPET_COUNT, SNIPPET_MAX_CHARS, STYLE_PROFILE_VERSION};

/// Stage budgets (T075 §3) and the style-extraction temperature.
pub const STAGE1_MAX_TOKENS: u32 = 400;
pub const STAGE2_MAX_TOKENS: u32 = 600;
pub const STYLE_TEMPERATURE: f32 = 0.2;

/// The six style dimensions (F_E5 §4.2). Each text field is a short free-text
/// description (≤ ~80 words, enforced by prompt).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StyleSummary {
    pub overall_tone: String,
    pub opening_patterns: Vec<String>,
    pub closing_patterns: Vec<String>,
    pub sentence_length: String,
    pub vocabulary: String,
    pub format_habit: String,
}

/// The persisted style profile — stored verbatim in
/// `account_ai_settings.style_profile` and read back by T076 prompt injection
/// and the settings UI. Keys are deliberately `snake_case` (the stored schema
/// of F_E5 §4.2), unlike the camelCase event payloads below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StyleProfileJson {
    pub version: u32,
    pub account_id: String,
    /// Unix timestamp (seconds) — drives the 30-day refresh check.
    pub generated_at: i64,
    pub summary: StyleSummary,
    /// Local-audit snippets (≤ 3 × 100 chars). Never injected into prompts.
    pub sample_snippets: Vec<String>,
    /// Set by the user when they hand-edit the summary (F_E5 §4.5); a pinned
    /// profile is never overwritten by recomputes.
    #[serde(default)]
    pub pinned: bool,
}

// ── style:* event payloads (T075 §3) ─────────────────────────────────────────

/// `style:progress` — one emission per stage: sampling → profiling → done.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StyleProgressPayload {
    pub account_id: String,
    /// "sampling" | "profiling" | "done"
    pub stage: String,
    pub pct: u8,
}

/// `style:done` — the profile was written (or, for a pinned profile, the
/// sample count was refreshed).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StyleDonePayload {
    pub account_id: String,
    pub sample_count: i64,
}

/// `style:error` — the run failed; `code` follows the standard wire table
/// (insufficient samples surfaces as `VALIDATION`, AI_MODES §6.7).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StyleErrorPayload {
    pub account_id: String,
    pub code: ErrorCode,
}

// ── Prompts ──────────────────────────────────────────────────────────────────

/// The JSON schema both stages must produce. Shared verbatim so stage 2 can
/// never drift from stage 1.
const SUMMARY_SCHEMA: &str = r#"{"overall_tone": string, "opening_patterns": [string], "closing_patterns": [string], "sentence_length": string, "vocabulary": string, "format_habit": string}"#;

fn stage1_system() -> String {
    format!(
        "You are a writing-style analyst. The following sent emails were all written by the \
         same person. Extract their writing-style features. Respond with only a single JSON \
         object — no markdown fences, no commentary — matching exactly this schema: \
         {SUMMARY_SCHEMA}. Keep every text field concise (under 80 words). \
         opening_patterns and closing_patterns must each list 3 to 5 frequent phrases."
    )
}

fn stage2_system() -> String {
    format!(
        "You are a writing-style analyst. The following JSON array holds partial style \
         analyses that all describe the same author. Merge them into one final summary. \
         Respond with only a single JSON object — no markdown fences, no commentary — \
         matching exactly this schema: {SUMMARY_SCHEMA}. Keep every text field concise \
         (under 80 words). opening_patterns and closing_patterns must each list 3 to 5 \
         frequent phrases."
    )
}

/// Render one sample group as the stage-1 user turn.
fn group_prompt(group: &[StyleSample]) -> String {
    let mut out = String::new();
    for (i, s) in group.iter().enumerate() {
        out.push_str(&format!(
            "Email {}\nSubject: {}\nBody:\n{}\n---\n",
            i + 1,
            s.subject,
            s.body_text_trimmed
        ));
    }
    out
}

fn user_message(content: String) -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content,
    }
}

/// Parse a model response into a [`StyleSummary`]. Tolerates stray prose or
/// markdown fences around the object; the error path is content-free (only the
/// response length is reported, 09 §5).
fn parse_summary(text: &str) -> AppResult<StyleSummary> {
    let start = text.find('{');
    let end = text.rfind('}');
    let slice = match (start, end) {
        (Some(s), Some(e)) if e > s => &text[s..=e],
        _ => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "style summary response contained no JSON object ({} chars)",
                text.chars().count()
            )))
        }
    };
    serde_json::from_str(slice).map_err(|_| {
        AppError::Internal(anyhow::anyhow!(
            "style summary response was not valid summary JSON ({} chars)",
            text.chars().count()
        ))
    })
}

// ── Build ────────────────────────────────────────────────────────────────────

/// Produce the final [`StyleProfileJson`] from the sampled corpus via the
/// two-stage chat flow. Fails with `VALIDATION` below
/// [`MIN_SAMPLES`](super::MIN_SAMPLES) samples (cold start, AI_MODES §6.7).
pub async fn build_style_profile(
    account_id: &str,
    model: &str,
    samples: &[StyleSample],
    client: &dyn AiProviderClient,
) -> AppResult<StyleProfileJson> {
    if samples.len() < MIN_SAMPLES {
        return Err(AppError::Validation(format!(
            "insufficient sent mail samples for style learning: {} found, {MIN_SAMPLES} required",
            samples.len()
        )));
    }
    let started = std::time::Instant::now();

    // Stage 1 — one partial summary per group of GROUP_SIZE samples.
    let mut partials: Vec<StyleSummary> = Vec::new();
    for group in samples.chunks(GROUP_SIZE) {
        let req = ChatRequest {
            model: model.to_string(),
            system: stage1_system(),
            messages: vec![user_message(group_prompt(group))],
            max_tokens: STAGE1_MAX_TOKENS,
            temperature: STYLE_TEMPERATURE,
            stop: Vec::new(),
            purpose: Capability::StyleProfile,
            request_id: Uuid::new_v4(),
        };
        let resp = client.chat(req).await.map_err(AppError::from)?;
        partials.push(parse_summary(&resp.text)?);
    }

    // Stage 2 — merge all partials into the final six-dimension summary.
    let partials_json = serde_json::to_string(&partials)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize partial summaries: {e}")))?;
    let req = ChatRequest {
        model: model.to_string(),
        system: stage2_system(),
        messages: vec![user_message(partials_json)],
        max_tokens: STAGE2_MAX_TOKENS,
        temperature: STYLE_TEMPERATURE,
        stop: Vec::new(),
        purpose: Capability::StyleProfile,
        request_id: Uuid::new_v4(),
    };
    let resp = client.chat(req).await.map_err(AppError::from)?;
    let summary = parse_summary(&resp.text)?;

    // Local-audit snippets: ≤ 3 short excerpts spread across the corpus.
    let sample_snippets: Vec<String> = uniform_indices(samples.len(), SNIPPET_COUNT)
        .into_iter()
        .map(|i| truncate_chars(&samples[i].body_text_trimmed, SNIPPET_MAX_CHARS))
        .collect();

    tracing::info!(
        event = "style_profile_built",
        account_id = account_id,
        sample_count = samples.len(),
        group_count = samples.len().div_ceil(GROUP_SIZE),
        latency_ms = started.elapsed().as_millis() as u64,
        "two-stage style summary complete"
    );

    Ok(StyleProfileJson {
        version: STYLE_PROFILE_VERSION,
        account_id: account_id.to_string(),
        generated_at: now_unix(),
        summary,
        sample_snippets,
        pinned: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::mock::MockProvider;
    use crate::ai::types::{ChatResponse, FinishReason, TokenUsage};
    use crate::ai::ProviderError;
    use crate::types::AiProvider;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    /// Realistic sent-mail corpus for profiler tests (English, no filler text).
    fn make_samples(n: usize) -> Vec<StyleSample> {
        let bodies = [
            "Hi Daniel,\n\nThanks for sending over the revised statement of work. I went \
             through the redlines this morning and the payment schedule now matches what we \
             agreed on the call. Two small things: the delivery date in section 3 still says \
             March, and the warranty clause should reference the master agreement.\n\nCould \
             you turn a clean version around by Thursday?\n\nBest regards,\nMaya",
            "Hi Priya,\n\nQuick update on the quarterly filing. The auditors confirmed the \
             inventory adjustment, so we are clear to close the books on Friday. I attached \
             the reconciliation summary for your records.\n\nLet me know if anything is \
             unclear.\n\nBest regards,\nMaya",
            "Hi Tom,\n\nGood catch on the invoice discrepancy. I checked with accounts \
             payable and the duplicate entry has been reversed. The corrected statement \
             should reach the vendor by end of week.\n\nThanks again for flagging it so \
             quickly.\n\nTalk soon,\nMaya",
            "Hi Elena,\n\nHope your week is going well. I reviewed the onboarding checklist \
             for the new contractor and everything looks complete except the NDA, which is \
             still waiting on a countersignature. I will chase legal tomorrow \
             morning.\n\nBest regards,\nMaya",
        ];
        (0..n)
            .map(|i| StyleSample {
                mail_id: format!("mail-{i}"),
                date_sent: 1_700_000_000 + i as i64 * 86_400,
                subject: format!("Re: Vendor contract update {i}"),
                body_text_trimmed: format!("{}\n\nRef {i}.", bodies[i % bodies.len()]),
            })
            .collect()
    }

    fn summary_json() -> String {
        r#"{"overall_tone":"Warm but direct; gets to the point within two sentences.","opening_patterns":["Hi {name},","Thanks for the quick turnaround","Hope your week is going well"],"closing_patterns":["Best regards,","Let me know if anything is unclear.","Talk soon,"],"sentence_length":"12-18 words on average","vocabulary":"Plain business English with contract terms such as SOW and redline","format_habit":"Short paragraphs of one to three sentences; bullet lists for action items."}"#
            .to_string()
    }

    fn ok_response(text: String) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text,
            finish: FinishReason::Stop,
            usage: TokenUsage::default(),
            model_echo: "mock-model".into(),
            latency_ms: 1,
        })
    }

    #[tokio::test]
    async fn valid_responses_produce_a_complete_profile() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        // 20 samples → one stage-1 group + one stage-2 merge.
        mock.push_chat(ok_response(summary_json()));
        mock.push_chat(ok_response(summary_json()));

        let samples = make_samples(20);
        let profile = build_style_profile("acc-1", "gpt-4o", &samples, &mock)
            .await
            .unwrap();

        assert_eq!(mock.chat_call_count(), 2);
        assert_eq!(profile.version, STYLE_PROFILE_VERSION);
        assert_eq!(profile.account_id, "acc-1");
        assert!(profile.generated_at > 0);
        assert!(!profile.pinned);
        // All six dimensions populated, deterministic given the scripted mock.
        assert_eq!(
            profile.summary.overall_tone,
            "Warm but direct; gets to the point within two sentences."
        );
        assert_eq!(profile.summary.opening_patterns.len(), 3);
        assert_eq!(profile.summary.closing_patterns.len(), 3);
        assert!(!profile.summary.sentence_length.is_empty());
        assert!(!profile.summary.vocabulary.is_empty());
        assert!(!profile.summary.format_habit.is_empty());
        // Snippets: ≤ 3, each capped at 100 chars, never empty.
        assert_eq!(profile.sample_snippets.len(), SNIPPET_COUNT);
        assert!(profile
            .sample_snippets
            .iter()
            .all(|s| !s.is_empty() && s.chars().count() <= SNIPPET_MAX_CHARS));
        // Persisted shape uses the stored snake_case schema of F_E5 §4.2.
        let wire = serde_json::to_value(&profile).unwrap();
        assert!(wire.get("sample_snippets").is_some());
        assert!(wire["summary"].get("opening_patterns").is_some());
    }

    #[tokio::test]
    async fn fenced_json_response_still_parses() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        let fenced = format!("```json\n{}\n```", summary_json());
        mock.push_chat(ok_response(fenced.clone()));
        mock.push_chat(ok_response(fenced));

        let profile = build_style_profile("acc-2", "gpt-4o", &make_samples(20), &mock)
            .await
            .unwrap();
        assert_eq!(profile.summary.opening_patterns.len(), 3);
    }

    #[tokio::test]
    async fn malformed_response_is_internal_error_not_panic() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(ok_response(
            "The author writes warmly and signs off politely.".into(),
        ));
        let err = build_style_profile("acc-3", "gpt-4o", &make_samples(20), &mock)
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Internal);
    }

    #[tokio::test]
    async fn fifteen_samples_fail_validation_with_count() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        let err = build_style_profile("acc-4", "gpt-4o", &make_samples(15), &mock)
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
        assert!(err.to_string().contains("15"));
        assert_eq!(mock.chat_call_count(), 0, "no chat call below the floor");
    }

    #[tokio::test]
    async fn provider_unreachable_maps_to_wire_code() {
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(Err(ProviderError::Unreachable("dns failure".into())));
        let err = build_style_profile("acc-5", "gpt-4o", &make_samples(20), &mock)
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::AiProviderUnreachable);
    }

    // ── log safety (09 §5) ───────────────────────────────────────────────────

    /// Shared in-memory sink for the capture subscriber.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    #[tokio::test]
    async fn logs_never_carry_mail_content() {
        const BODY_SENTINEL: &str = "the settlement amount for Holt v. Marsh is confidential";
        const SUBJECT_SENTINEL: &str = "Holt v. Marsh settlement terms";

        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(capture.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let mut samples = make_samples(20);
        for s in &mut samples {
            s.subject = SUBJECT_SENTINEL.to_string();
            s.body_text_trimmed = format!(
                "Hi Counsel,\n\nAs discussed, {BODY_SENTINEL}. Please keep this between us \
                 until the filing is public.\n\nBest regards,\nMaya"
            );
        }

        // Run both the success path and a parse-failure path under capture.
        let mock = MockProvider::healthy(AiProvider::Openai);
        mock.push_chat(ok_response(summary_json()));
        mock.push_chat(ok_response(summary_json()));
        build_style_profile("acc-log", "gpt-4o", &samples, &mock)
            .await
            .unwrap();
        let mock2 = MockProvider::healthy(AiProvider::Openai);
        mock2.push_chat(ok_response(format!("Summary of {BODY_SENTINEL}")));
        let _ = build_style_profile("acc-log", "gpt-4o", &samples, &mock2).await;

        let logs = String::from_utf8_lossy(&capture.0.lock().unwrap()).to_string();
        assert!(!logs.contains(BODY_SENTINEL), "body text leaked into logs");
        assert!(!logs.contains(SUBJECT_SENTINEL), "subject leaked into logs");
        assert!(
            !logs.contains("body_text"),
            "raw body field leaked into logs"
        );
    }
}
