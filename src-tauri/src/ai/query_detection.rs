//! I3 proactive-query trigger detection (T095, F_I3 §2).
//!
//! Pure, DB-free rules so they unit-test without a database or an AI provider.
//! T1/T2/T4/T5 are rule-based here; T3 (multi-path) and T6 (rule-boundary) are
//! AI-assisted in the spec and are intentionally gated stubs in v0.6 — see
//! [`detect_query_triggers`] — until the provider call is wired (no reliable
//! rule exists for them without an LLM). The caller ([`crate::ai::pipeline::i3_stage`])
//! supplies the loaded context and persists whatever this returns.

/// Risk keywords that make an unknown sender worth a T1 identity check. Lowercase;
/// matched as substrings against the lowercased body (F_I3 §2 initial set).
pub const RISK_KEYWORDS: &[&str] = &[
    "contract",
    "agreement",
    "payment",
    "invoice",
    "wire transfer",
    "bank details",
    "bank account",
    "nda",
    "confidential",
    "purchase order",
];

/// Phrases that signal a scheduling request (T2).
const MEETING_PHRASES: &[&str] = &[
    "meeting",
    "schedule a call",
    "schedule a meeting",
    "let's meet",
    "are you available",
    "when can you",
    "set up a call",
    "book a time",
];

/// Phrases that reference an attachment (T5 — paired with `has_attachments`).
const ATTACHMENT_PHRASES: &[&str] = &[
    "see attachment",
    "see attached",
    "attached file",
    "attached please find",
    "please find attached",
    "enclosed",
    "the attachment",
];

/// Phrases that reference prior context (T5 — paired with a reply chain).
const PRIOR_CONTEXT_PHRASES: &[&str] = &[
    "as discussed",
    "as we agreed",
    "per our conversation",
    "as mentioned earlier",
];

const WEEKDAYS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "tomorrow",
    "today",
    "next week",
];

/// Which trigger types are enabled for the account (`account_ai_settings.tN_enabled`).
#[derive(Debug, Clone, Copy)]
pub struct TriggerFlags {
    pub t1: bool,
    pub t2: bool,
    pub t3: bool,
    pub t4: bool,
    pub t5: bool,
    pub t6: bool,
}

impl Default for TriggerFlags {
    fn default() -> Self {
        // Mirrors the schema defaults (t5 off, the rest on).
        Self {
            t1: true,
            t2: true,
            t3: true,
            t4: true,
            t5: false,
            t6: true,
        }
    }
}

/// The loaded facts the rules need, assembled by `i3_stage` (DB-free here).
#[derive(Debug, Clone)]
pub struct DetectionInput {
    pub body: String,
    /// `contacts.interaction_count == 0` for the sender.
    pub is_new_sender: bool,
    pub has_attachments: bool,
    /// The mail has an `in_reply_to` (it is part of an existing thread).
    pub has_reply_context: bool,
    /// `risk_events.id` of an open level-4 (T4) event for this mail, if any (E4).
    pub t4_risk_event_id: Option<String>,
}

/// One detected trigger. `priority` follows F_I3 §3.2 (T3/T4 = 1, T1/T2/T5 = 3,
/// T6 = 5); lower is more urgent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryTrigger {
    pub trigger_type: String,
    pub priority: i64,
    pub risk_event_id: Option<String>,
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// True if the text names a concrete time (a `H:MM` clock value or a weekday /
/// relative-day word) — used to decide whether a meeting request is missing its
/// time (T2).
fn has_concrete_time(body_lc: &str) -> bool {
    if contains_any(body_lc, WEEKDAYS) {
        return true;
    }
    // Scan for a digit ':' digit pattern (e.g. "3:30").
    let bytes = body_lc.as_bytes();
    for i in 1..bytes.len().saturating_sub(1) {
        if bytes[i] == b':' && bytes[i - 1].is_ascii_digit() && bytes[i + 1].is_ascii_digit() {
            return true;
        }
    }
    false
}

/// Detect every applicable trigger for one inbound mail (F_I3 §2). Order is
/// urgency-first (T4 before the rest) but the caller decides the primary.
pub fn detect_query_triggers(input: &DetectionInput, flags: &TriggerFlags) -> Vec<QueryTrigger> {
    let mut out = Vec::new();
    let body_lc = input.body.to_lowercase();

    // T4 — risk pre-scan hit from E4 (highest priority; never expires).
    if flags.t4 {
        if let Some(id) = &input.t4_risk_event_id {
            out.push(QueryTrigger {
                trigger_type: "T4".into(),
                priority: 1,
                risk_event_id: Some(id.clone()),
            });
        }
    }

    // T1 — unknown sender raising a risk-laden topic.
    if flags.t1 && input.is_new_sender && contains_any(&body_lc, RISK_KEYWORDS) {
        out.push(QueryTrigger {
            trigger_type: "T1".into(),
            priority: 3,
            risk_event_id: None,
        });
    }

    // T2 — a scheduling request with no concrete time.
    if flags.t2 && contains_any(&body_lc, MEETING_PHRASES) && !has_concrete_time(&body_lc) {
        out.push(QueryTrigger {
            trigger_type: "T2".into(),
            priority: 3,
            risk_event_id: None,
        });
    }

    // T5 — references an attachment that isn't there, or prior context with no
    // reply chain to ground it.
    if flags.t5 {
        let missing_attachment =
            contains_any(&body_lc, ATTACHMENT_PHRASES) && !input.has_attachments;
        let missing_context =
            contains_any(&body_lc, PRIOR_CONTEXT_PHRASES) && !input.has_reply_context;
        if missing_attachment || missing_context {
            out.push(QueryTrigger {
                trigger_type: "T5".into(),
                priority: 3,
                risk_event_id: None,
            });
        }
    }

    // T3 (multi-path) and T6 (rule boundary) are AI-assisted (F_I3 §2). Gated but
    // not rule-detectable without a provider call; deferred to a later card so
    // detection stays deterministic and provider-free in v0.6.
    let _ = (flags.t3, flags.t6);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(body: &str) -> DetectionInput {
        DetectionInput {
            body: body.into(),
            is_new_sender: false,
            has_attachments: false,
            has_reply_context: true,
            t4_risk_event_id: None,
        }
    }

    #[test]
    fn t1_fires_for_new_sender_with_risk_keyword() {
        let mut i = input("Please review the attached contract and confirm payment terms.");
        i.is_new_sender = true;
        i.has_attachments = true; // attachment present → no T5
        let triggers = detect_query_triggers(&i, &TriggerFlags::default());
        assert!(triggers.iter().any(|t| t.trigger_type == "T1"));
    }

    #[test]
    fn t1_skipped_for_known_sender() {
        let i = input("Please review the attached contract."); // is_new_sender = false
        let triggers = detect_query_triggers(&input(&i.body), &TriggerFlags::default());
        assert!(!triggers.iter().any(|t| t.trigger_type == "T1"));
    }

    #[test]
    fn t1_respects_disable_flag() {
        let mut i = input("contract payment");
        i.is_new_sender = true;
        let flags = TriggerFlags {
            t1: false,
            ..TriggerFlags::default()
        };
        assert!(!detect_query_triggers(&i, &flags)
            .iter()
            .any(|t| t.trigger_type == "T1"));
    }

    #[test]
    fn t2_fires_without_a_concrete_time_and_not_with_one() {
        let no_time = input("Could we schedule a call to discuss next steps?");
        assert!(detect_query_triggers(&no_time, &TriggerFlags::default())
            .iter()
            .any(|t| t.trigger_type == "T2"));

        let with_time = input("Could we schedule a call on Tuesday?");
        assert!(!detect_query_triggers(&with_time, &TriggerFlags::default())
            .iter()
            .any(|t| t.trigger_type == "T2"));

        let with_clock = input("Could we schedule a call at 3:30 please?");
        assert!(
            !detect_query_triggers(&with_clock, &TriggerFlags::default())
                .iter()
                .any(|t| t.trigger_type == "T2")
        );
    }

    #[test]
    fn t5_fires_for_missing_attachment() {
        let flags = TriggerFlags {
            t5: true,
            ..TriggerFlags::default()
        };
        let mut i = input("Please find attached the signed form.");
        i.has_attachments = false;
        assert!(detect_query_triggers(&i, &flags)
            .iter()
            .any(|t| t.trigger_type == "T5"));
        // With the attachment actually present, no T5.
        i.has_attachments = true;
        assert!(!detect_query_triggers(&i, &flags)
            .iter()
            .any(|t| t.trigger_type == "T5"));
    }

    #[test]
    fn t4_fires_from_risk_event_with_priority_one() {
        let mut i = input("anything");
        i.t4_risk_event_id = Some("risk-1".into());
        let triggers = detect_query_triggers(&i, &TriggerFlags::default());
        let t4 = triggers.iter().find(|t| t.trigger_type == "T4").unwrap();
        assert_eq!(t4.priority, 1);
        assert_eq!(t4.risk_event_id.as_deref(), Some("risk-1"));
    }
}
