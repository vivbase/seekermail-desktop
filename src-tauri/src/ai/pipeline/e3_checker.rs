//! E3 six-point pre-send self-check (T085 §3, F_E3 §4.2). Pure functions —
//! no DB, no LLM — so every rule is unit-testable in isolation.
//!
//! 1. **Structure** — body length 30..=2000 chars with real content.
//! 2. **Recipients** — no CC added when the original had none.
//! 3. **Content** — no monetary figures the original mail never mentioned
//!    (shares the E4 amount regex).
//! 4. **Style** — `check_style_drift` (T076 stub: always within bounds; the
//!    real scorer activates automatically when T076 lands).
//! 5. **Blocked terms** — built-in absolutes (`guarantee`/`absolutely` plus
//!    the spec's CJK equivalents via `\u{..}` escapes) + user terms from
//!    `app_settings['ai.e3_blocked_terms']`.
//! 6. **Self-reference** — Jaccard word-set similarity between draft and
//!    original > 0.5 means the model echoed the input instead of replying.
//!
//! Any violation demotes the draft to E2 review — never a silent send.

use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::ai::style::{check_style_drift, StyleProfileJson};

use super::e4_classifier::mentions_amount;
use super::PipelineMail;

/// `app_settings` key for the user-managed blocked-term list (JSON array).
pub const E3_BLOCKED_TERMS_KEY: &str = "ai.e3_blocked_terms";

/// Draft body length window (chars), F_E3 §4.2.
pub const DRAFT_MIN_CHARS: usize = 30;
pub const DRAFT_MAX_CHARS: usize = 2_000;
/// Draft/original similarity ceiling — above this the draft is an echo.
pub const SELF_REFERENCE_JACCARD_MAX: f64 = 0.5;

/// Built-in over-commitment terms (English + the spec's CJK equivalents
/// `\u{4FDD}\u{8BC1}` "guarantee" and `\u{7EDD}\u{5BF9}` "absolutely",
/// expressed as escapes — no raw CJK in source).
static BLOCKED_TERMS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bguarantee\b|\babsolutely\b|\u{4FDD}\u{8BC1}|\u{7EDD}\u{5BF9}")
        .expect("blocked terms regex is valid")
});

/// One failed check. Identifier-style names only — these reach logs and the
/// downgrade audit row (09 §5: never include the offending text itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckViolation {
    Structure,
    AddedRecipients,
    UnpromptedFigures,
    StyleDrift,
    BlockedTerm,
    SelfReference,
}

impl CheckViolation {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckViolation::Structure => "structure",
            CheckViolation::AddedRecipients => "added_recipients",
            CheckViolation::UnpromptedFigures => "unprompted_figures",
            CheckViolation::StyleDrift => "style_drift",
            CheckViolation::BlockedTerm => "blocked_term",
            CheckViolation::SelfReference => "self_reference",
        }
    }
}

/// Word-set Jaccard similarity (case-insensitive, alphanumeric tokens).
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let tokens = |s: &str| -> HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let set_a = tokens(a);
    let set_b = tokens(b);
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = set_a.union(&set_b).count() as f64;
    intersection / union
}

/// Run all six checks on a generated draft. Empty result = safe to queue for
/// auto-send; any entry demotes to E2 review.
pub fn check_draft(
    draft_body: &str,
    draft_cc_count: usize,
    orig_mail: &PipelineMail,
    style: Option<&StyleProfileJson>,
    user_blocked_terms: &[String],
) -> Vec<CheckViolation> {
    let mut violations = Vec::new();

    // 1) Structure: length window + at least one non-empty line.
    let char_count = draft_body.chars().count();
    let has_content = draft_body.lines().any(|l| !l.trim().is_empty());
    if !(DRAFT_MIN_CHARS..=DRAFT_MAX_CHARS).contains(&char_count) || !has_content {
        violations.push(CheckViolation::Structure);
    }

    // 2) Recipients: never add CC when the original had none. (The shared
    // generation path always writes cc_addrs = '[]', so this guards future
    // callers that pass an enriched draft.)
    if orig_mail.cc_count() == 0 && draft_cc_count > 0 {
        violations.push(CheckViolation::AddedRecipients);
    }

    // 3) Content: monetary figures the original never mentioned.
    if mentions_amount(draft_body) && !mentions_amount(orig_mail.text()) {
        violations.push(CheckViolation::UnpromptedFigures);
    }

    // 4) Style drift (T076 stub today — wired so the real scorer activates).
    if let Some(profile) = style {
        if !check_style_drift(draft_body, profile).within_bounds {
            violations.push(CheckViolation::StyleDrift);
        }
    }

    // 5) Blocked terms: built-in absolutes + user-configured list.
    let lower = draft_body.to_lowercase();
    let user_hit = user_blocked_terms
        .iter()
        .filter(|t| !t.trim().is_empty())
        .any(|t| lower.contains(&t.trim().to_lowercase()));
    if BLOCKED_TERMS_RE.is_match(draft_body) || user_hit {
        violations.push(CheckViolation::BlockedTerm);
    }

    // 6) Self-reference: the draft must not be an echo of the original.
    if jaccard_similarity(draft_body, orig_mail.text()) > SELF_REFERENCE_JACCARD_MAX {
        violations.push(CheckViolation::SelfReference);
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orig(body: &str) -> PipelineMail {
        PipelineMail {
            id: "m1".into(),
            account_id: "acc".into(),
            thread_id: None,
            subject: "Renewal terms".into(),
            from_email: "daniel@vendorco.example".into(),
            to_addrs: "[]".into(),
            cc_addrs: "[]".into(),
            body_text: Some(body.into()),
            snippet: None,
            imap_flags: "[]".into(),
            spam_score: None,
            has_attachments: 0,
            is_sent: 0,
        }
    }

    const GOOD_DRAFT: &str = "Hi Daniel,\n\nThanks for the update. The plan works for us and \
we will follow up with the next steps this week.\n\nBest,\nMaya";

    #[test]
    fn clean_draft_passes_all_checks() {
        let violations = check_draft(
            GOOD_DRAFT,
            0,
            &orig("Could you confirm the renewal terms?"),
            None,
            &[],
        );
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn empty_or_oversized_body_violates_structure() {
        let mail = orig("Original text.");
        assert!(check_draft("", 0, &mail, None, &[]).contains(&CheckViolation::Structure));
        assert!(check_draft("Too short.", 0, &mail, None, &[]).contains(&CheckViolation::Structure));
        let huge = "word ".repeat(500);
        assert!(check_draft(&huge, 0, &mail, None, &[]).contains(&CheckViolation::Structure));
    }

    #[test]
    fn added_cc_violates_recipients() {
        let mail = orig("Original text without cc.");
        assert!(
            check_draft(GOOD_DRAFT, 1, &mail, None, &[]).contains(&CheckViolation::AddedRecipients)
        );
        // Original already had CC → adding is allowed.
        let mut with_cc = mail;
        with_cc.cc_addrs = r#"[{"name":"","email":"cc@x.y"}]"#.into();
        assert!(!check_draft(GOOD_DRAFT, 1, &with_cc, None, &[])
            .contains(&CheckViolation::AddedRecipients));
    }

    #[test]
    fn unprompted_money_figure_is_a_violation() {
        let mail = orig("Could you confirm the renewal terms?");
        let draft =
            "Hi Daniel,\n\nWe can settle this for $50,000 by Friday if that works.\n\nBest,\nMaya";
        assert!(
            check_draft(draft, 0, &mail, None, &[]).contains(&CheckViolation::UnpromptedFigures)
        );
        // The original mentioned a figure → the reply may repeat one.
        let asked = orig("The invoice totals $50,000 — can you confirm?");
        assert!(
            !check_draft(draft, 0, &asked, None, &[]).contains(&CheckViolation::UnpromptedFigures)
        );
    }

    #[test]
    fn blocked_terms_builtin_and_user_list() {
        let mail = orig("Original text.");
        let draft = "Hi Daniel,\n\nI guarantee this will be resolved before the deadline.\n\nBest,";
        assert!(check_draft(draft, 0, &mail, None, &[]).contains(&CheckViolation::BlockedTerm));
        let custom =
            "Hi Daniel,\n\nOur competitor MegaCorp cannot match this offer at all.\n\nBest,";
        assert!(check_draft(draft, 0, &mail, None, &["megacorp".into()])
            .contains(&CheckViolation::BlockedTerm));
        assert!(check_draft(custom, 0, &mail, None, &["megacorp".into()])
            .contains(&CheckViolation::BlockedTerm));
        assert!(
            !check_draft(GOOD_DRAFT, 0, &mail, None, &["megacorp".into()])
                .contains(&CheckViolation::BlockedTerm)
        );
    }

    #[test]
    fn echoed_draft_violates_self_reference() {
        let body = "Could you confirm the renewal terms we discussed last week before Friday?";
        let mail = orig(body);
        let echo = format!("{body} Could you confirm the renewal terms we discussed?");
        assert!(check_draft(&echo, 0, &mail, None, &[]).contains(&CheckViolation::SelfReference));
        assert!(
            !check_draft(GOOD_DRAFT, 0, &mail, None, &[]).contains(&CheckViolation::SelfReference)
        );
    }

    #[test]
    fn jaccard_basics() {
        assert!(jaccard_similarity("a b c", "a b c") > 0.99);
        assert_eq!(jaccard_similarity("", "anything"), 0.0);
        assert!(jaccard_similarity("alpha beta", "gamma delta") < 0.01);
    }
}
