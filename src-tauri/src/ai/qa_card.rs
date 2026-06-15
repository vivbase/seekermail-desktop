//! Structured QA-card content schema + generation (T098, F_I4 §4).
//!
//! The `im_messages.content` for a `query_card` message is a JSON [`QaCardContent`].
//! [`generate_qa_card_content`] produces a spec-compliant option set per trigger
//! type; [`validate_qa_card_content`] guards the invariants the T099 frontend
//! relies on. These structs are specta-exported so the frontend shares the shape.
//!
//! Note on limits: the card text proposed a 15-codepoint option-label cap, but its
//! own example labels ("Confirm and proceed") exceed that, so we use a realistic
//! [`MAX_LABEL_LEN`] = 40 (truncated with an ellipsis). The validated invariants
//! are the ones the UI actually depends on: 2–4 options, an ≤80 question, and a
//! mandatory "view original email" option for T4 (F_I3 §7).

use serde::{Deserialize, Serialize};
use specta::Type;

/// Current card schema version.
pub const CARD_VERSION: u32 = 1;
/// Max question length in UTF-16 code units (F_I4 §2.1).
pub const MAX_QUESTION_LEN: usize = 80;
/// Max option-label length (see module note).
pub const MAX_LABEL_LEN: usize = 40;
/// The skip option's stable value.
pub const SKIP_VALUE: &str = "__skip__";
/// The T4 "view original email" option's stable value (F_I3 §7).
pub const VIEW_EMAIL_VALUE: &str = "__view_email__";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QaCardOption {
    pub id: String,
    pub label: String,
    pub value: String,
}

impl QaCardOption {
    fn new(id: &str, label: &str, value: &str) -> Self {
        Self {
            id: id.into(),
            label: truncate(label, MAX_LABEL_LEN),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QaCardSubQuestion {
    pub question_text: String,
    pub options: Vec<QaCardOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QaCardResponse {
    pub selected_option_ids: Vec<String>,
    pub free_text: Option<String>,
    pub submitted_at: i64,
    pub action_result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct QaCardContent {
    pub card_version: u32,
    /// `pending_queries.id` this card resolves (used by T096 answer/skip).
    pub linked_query_id: String,
    pub trigger_type: String,
    /// `high` | `normal` | `low`.
    pub priority: String,
    pub linked_email_id: Option<String>,
    pub question_text: String,
    pub options: Vec<QaCardOption>,
    pub multi_select: bool,
    pub free_text_placeholder: Option<String>,
    pub sub_questions: Vec<QaCardSubQuestion>,
    pub response: Option<QaCardResponse>,
}

/// Validation failure (mapped to `AppError::Validation` by callers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaCardValidationError(pub String);

/// UTF-16-aware truncation with an ellipsis (matches the frontend's codepoint view).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn priority_label(priority: i64) -> &'static str {
    if priority <= 1 {
        "high"
    } else if priority <= 3 {
        "normal"
    } else {
        "low"
    }
}

/// Trigger-specific option set (F_I4 §3). Every list ends with Skip — except T4,
/// whose mandatory "View original email" is last and Skip is second-to-last
/// (F_I3 §7); and T6, whose terminal "Decline" carries the skip value.
fn options_for(trigger_type: &str) -> Vec<QaCardOption> {
    let skip = QaCardOption::new("opt_skip", "Skip", SKIP_VALUE);
    match trigger_type {
        "T1" => vec![
            QaCardOption::new("opt_known", "Yes, I know them", "known"),
            QaCardOption::new("opt_unknown", "No, treat as unknown", "unknown"),
            skip,
        ],
        "T2" => vec![
            QaCardOption::new("opt_propose", "Propose meeting times", "propose_times"),
            QaCardOption::new("opt_self", "I'll reply myself", "manual"),
            skip,
        ],
        "T3" => vec![
            QaCardOption::new("opt_a", "Option A", "path_a"),
            QaCardOption::new("opt_b", "Option B", "path_b"),
            skip,
        ],
        "T4" => vec![
            QaCardOption::new("opt_confirm", "Confirm and proceed", "confirm"),
            QaCardOption::new("opt_block", "Block this email", "block"),
            skip,
            QaCardOption::new("opt_view_email", "View original email", VIEW_EMAIL_VALUE),
        ],
        "T5" => vec![
            QaCardOption::new("opt_request", "Request the attachment", "request"),
            QaCardOption::new("opt_have", "I have the file", "have_file"),
            QaCardOption::new("opt_ignore", "Ignore the reference", "ignore"),
            skip,
        ],
        "T6" => vec![
            QaCardOption::new("opt_authorize", "Authorize this exception", "authorize"),
            QaCardOption::new("opt_escalate", "Escalate for review", "escalate"),
            QaCardOption::new("opt_decline", "Decline and notify", SKIP_VALUE),
        ],
        _ => vec![QaCardOption::new("opt_ok", "Proceed", "proceed"), skip],
    }
}

/// Build the full card content for one trigger (T098).
pub fn generate_qa_card_content(
    trigger_type: &str,
    priority: i64,
    linked_query_id: &str,
    linked_email_id: &str,
    question_text: &str,
) -> QaCardContent {
    QaCardContent {
        card_version: CARD_VERSION,
        linked_query_id: linked_query_id.to_string(),
        trigger_type: trigger_type.to_string(),
        priority: priority_label(priority).to_string(),
        linked_email_id: Some(linked_email_id.to_string()),
        question_text: truncate(question_text, MAX_QUESTION_LEN),
        options: options_for(trigger_type),
        multi_select: false,
        free_text_placeholder: Some("Add a note (optional)".into()),
        sub_questions: Vec::new(),
        response: None,
    }
}

/// Validate the invariants the frontend depends on (F_I4 §2.1). Single-question
/// cards need 2–4 options; multi-question cards validate each sub-question instead.
pub fn validate_qa_card_content(content: &QaCardContent) -> Result<(), QaCardValidationError> {
    if content.question_text.chars().count() > MAX_QUESTION_LEN {
        return Err(QaCardValidationError(format!(
            "question_text exceeds {MAX_QUESTION_LEN} characters"
        )));
    }

    if content.sub_questions.is_empty() {
        validate_options(&content.options)?;
    } else {
        for sub in &content.sub_questions {
            validate_options(&sub.options)?;
        }
    }

    // T4 must always offer "view original email" (F_I3 §7).
    if content.trigger_type == "T4" && !content.options.iter().any(|o| o.value == VIEW_EMAIL_VALUE)
    {
        return Err(QaCardValidationError(
            "T4 card must include a 'view original email' option".into(),
        ));
    }
    Ok(())
}

fn validate_options(options: &[QaCardOption]) -> Result<(), QaCardValidationError> {
    if !(2..=4).contains(&options.len()) {
        return Err(QaCardValidationError(format!(
            "expected 2–4 options, got {}",
            options.len()
        )));
    }
    for o in options {
        if o.label.chars().count() > MAX_LABEL_LEN {
            return Err(QaCardValidationError(format!(
                "option label exceeds {MAX_LABEL_LEN} characters"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(trigger: &str, priority: i64) -> QaCardContent {
        generate_qa_card_content(
            trigger,
            priority,
            "q1",
            "m1",
            "Do you recognise this sender?",
        )
    }

    #[test]
    fn t4_card_has_view_email_and_skip_and_high_priority() {
        let c = gen("T4", 1);
        assert_eq!(c.priority, "high");
        assert!(c.options.iter().any(|o| o.value == VIEW_EMAIL_VALUE));
        assert!(c.options.iter().any(|o| o.value == SKIP_VALUE));
        // View-email is last, Skip second-to-last (F_I3 §7).
        assert_eq!(c.options.last().unwrap().value, VIEW_EMAIL_VALUE);
        validate_qa_card_content(&c).unwrap();
    }

    #[test]
    fn each_trigger_generates_2_to_4_options_and_validates() {
        for trigger in ["T1", "T2", "T3", "T4", "T5", "T6"] {
            let c = gen(trigger, 3);
            assert!((2..=4).contains(&c.options.len()), "{trigger} option count");
            validate_qa_card_content(&c).expect(trigger);
        }
    }

    #[test]
    fn validate_rejects_long_question() {
        let mut c = gen("T1", 3);
        c.question_text = "x".repeat(81);
        assert!(validate_qa_card_content(&c).is_err());
    }

    #[test]
    fn validate_rejects_too_few_options() {
        let mut c = gen("T1", 3);
        c.options.truncate(1);
        assert!(validate_qa_card_content(&c).is_err());
    }

    #[test]
    fn validate_rejects_t4_without_view_email() {
        let mut c = gen("T4", 1);
        c.options.retain(|o| o.value != VIEW_EMAIL_VALUE);
        assert!(validate_qa_card_content(&c).is_err());
    }

    #[test]
    fn long_question_is_truncated_by_generate() {
        let long = "y".repeat(200);
        let c = generate_qa_card_content("T1", 3, "q", "m", &long);
        assert!(c.question_text.chars().count() <= MAX_QUESTION_LEN);
        validate_qa_card_content(&c).unwrap();
    }
}
