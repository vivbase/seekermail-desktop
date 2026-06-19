//! E3 style-drift validation (T076 §3).
//!
//! [`check_style_drift`] compares a generated draft against the account's
//! learned style profile so E3's six-point self-check (T085, F_E3 §4.2) is
//! fully wired — a draft that breaks the operator's consistent greeting /
//! sign-off habits, or is otherwise grossly off-style (e.g. shouting in all
//! caps), is demoted to E2 human review instead of being auto-sent.
//!
//! The check is **deterministic and provider-free** on purpose: like the other
//! five checks in [`crate::ai::pipeline::e3_checker`] it must unit-test without
//! a database or an LLM. It is also intentionally *conservative* — it only
//! flags gross, structurally detectable drift, because a false positive merely
//! routes a good draft to review (safe), while a false negative would let a
//! genuinely off-style reply leave un-reviewed. Subtle tone / vocabulary
//! scoring is a later, model-assisted refinement and would need an AI call this
//! pure stage deliberately avoids.

use std::collections::HashSet;

use super::profiler::StyleProfileJson;

/// First / last non-empty lines scanned for a greeting / sign-off.
const EDGE_LINES: usize = 2;
/// All-caps guard: only fires once a body has at least this many letters, so a
/// short acronym-heavy reply is never mistaken for shouting.
const SHOUT_MIN_ALPHA: usize = 40;
/// Uppercase fraction (of letters) above which a long body reads as shouting.
const SHOUT_UPPER_RATIO: f32 = 0.6;

/// Generic salutation cues, supplementing the author's own learned opening
/// patterns so the check still works for accounts whose patterns yield no
/// usable tokens. Lowercase, whole-word matched.
const GREETING_CUES: &[&str] = &[
    "hi",
    "hello",
    "hey",
    "dear",
    "greetings",
    "good",
    "morning",
    "afternoon",
    "evening",
];
/// Generic sign-off cues, supplementing the author's learned closing patterns.
const SIGNOFF_CUES: &[&str] = &[
    "regards",
    "best",
    "thanks",
    "thank",
    "sincerely",
    "cheers",
    "warmly",
    "respectfully",
    "yours",
    "talk",
    "speak",
    "wishes",
];

/// Outcome of a style-drift check on a generated draft (T085 consumes it
/// before an autonomous send).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleDriftResult {
    /// `true` — the draft is close enough to the learned style to auto-send.
    pub within_bounds: bool,
    /// Why the draft drifted (stable identifier for logs/tests); `None` when
    /// `within_bounds` is `true`. Never carries draft text (09 §5).
    pub reason: Option<&'static str>,
}

impl StyleDriftResult {
    const OK: Self = Self {
        within_bounds: true,
        reason: None,
    };

    fn drift(reason: &'static str) -> Self {
        Self {
            within_bounds: false,
            reason: Some(reason),
        }
    }
}

/// Compare a generated draft against the learned profile (T076 §3). Returns
/// `within_bounds: false` only on gross, deterministically detectable drift;
/// every other draft passes so the auto-send path is never blocked spuriously.
pub fn check_style_drift(draft_text: &str, profile: &StyleProfileJson) -> StyleDriftResult {
    let draft = draft_text.trim();
    if draft.is_empty() {
        // An empty body is already a Structure violation upstream; never let the
        // style stage green-light the absence of any style.
        return StyleDriftResult::drift("empty_draft");
    }

    // Gross casing drift: a long, mostly-uppercase body is "shouting" — off
    // style for any normal correspondent regardless of the learned profile.
    if is_shouting(draft) {
        return StyleDriftResult::drift("all_caps");
    }

    let summary = &profile.summary;

    // Greeting habit: when the author consistently opens with a greeting, the
    // draft's first lines should carry a salutation cue (their own learned ones,
    // or a generic fallback).
    if author_has_habit(&summary.opening_patterns) {
        let cues = cue_set(&summary.opening_patterns, GREETING_CUES);
        if !edge_has_cue(draft, Edge::Head, &cues) {
            return StyleDriftResult::drift("missing_greeting");
        }
    }

    // Sign-off habit: symmetric to the greeting check, on the closing lines.
    if author_has_habit(&summary.closing_patterns) {
        let cues = cue_set(&summary.closing_patterns, SIGNOFF_CUES);
        if !edge_has_cue(draft, Edge::Tail, &cues) {
            return StyleDriftResult::drift("missing_signoff");
        }
    }

    StyleDriftResult::OK
}

/// Whether the learned profile records a habit worth enforcing (at least one
/// non-empty pattern).
fn author_has_habit(patterns: &[String]) -> bool {
    patterns.iter().any(|p| !p.trim().is_empty())
}

/// Which end of the draft a cue check looks at.
enum Edge {
    Head,
    Tail,
}

/// Significant alphabetic cue tokens drawn from the learned patterns (with
/// `{placeholder}` leftovers and 1-char noise dropped) unioned with the generic
/// fallback cues. All lowercase.
fn cue_set(patterns: &[String], fallback: &[&str]) -> HashSet<String> {
    let mut cues: HashSet<String> = fallback.iter().map(|s| s.to_string()).collect();
    for pattern in patterns {
        for token in pattern.split(|c: char| !c.is_alphabetic()) {
            let token = token.to_lowercase();
            // Skip placeholder leftovers ("name" from "{name}") and 1-char noise.
            if token.len() >= 2 && token != "name" {
                cues.insert(token);
            }
        }
    }
    cues
}

/// Does the first / last [`EDGE_LINES`] non-empty lines of the draft contain any
/// cue as a whole word? Whole-word (token) matching avoids false hits like
/// "hi" inside "think".
fn edge_has_cue(draft: &str, edge: Edge, cues: &HashSet<String>) -> bool {
    let lines: Vec<&str> = draft
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let edge_lines: Vec<&str> = match edge {
        Edge::Head => lines.iter().take(EDGE_LINES).copied().collect(),
        Edge::Tail => lines.iter().rev().take(EDGE_LINES).copied().collect(),
    };
    edge_lines
        .join(" ")
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .any(|word| cues.contains(&word.to_lowercase()))
}

/// A long body that is mostly uppercase letters reads as shouting.
fn is_shouting(draft: &str) -> bool {
    let alpha = draft.chars().filter(|c| c.is_alphabetic()).count();
    if alpha < SHOUT_MIN_ALPHA {
        return false;
    }
    let upper = draft.chars().filter(|c| c.is_uppercase()).count();
    (upper as f32 / alpha as f32) > SHOUT_UPPER_RATIO
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::style::profiler::StyleSummary;
    use crate::ai::style::STYLE_PROFILE_VERSION;

    /// A profile whose author consistently greets and signs off.
    fn profile_with_habits() -> StyleProfileJson {
        StyleProfileJson {
            version: STYLE_PROFILE_VERSION,
            account_id: "acc".into(),
            generated_at: 1_750_000_000,
            summary: StyleSummary {
                overall_tone: "Concise and courteous.".into(),
                opening_patterns: vec!["Hi {name},".into(), "Hello,".into()],
                closing_patterns: vec!["Best regards,".into(), "Talk soon,".into()],
                sentence_length: "short".into(),
                vocabulary: "plain".into(),
                format_habit: "short paragraphs".into(),
            },
            sample_snippets: Vec::new(),
            pinned: false,
        }
    }

    /// A terse author with no recorded greeting / sign-off habit.
    fn profile_no_habits() -> StyleProfileJson {
        let mut p = profile_with_habits();
        p.summary.opening_patterns = Vec::new();
        p.summary.closing_patterns = Vec::new();
        p
    }

    const GOOD_DRAFT: &str =
        "Hi Daniel,\n\nThanks for the update — the plan works and we will follow up this \
week.\n\nBest regards,\nMaya";

    #[test]
    fn on_style_draft_is_within_bounds() {
        let result = check_style_drift(GOOD_DRAFT, &profile_with_habits());
        assert!(result.within_bounds, "{result:?}");
        assert_eq!(result.reason, None);
    }

    #[test]
    fn missing_greeting_is_drift() {
        // No salutation in the opening lines, though the author always greets.
        let body = "The renewal terms are confirmed and we will proceed with the updated \
schedule.\n\nBest regards,\nMaya";
        let result = check_style_drift(body, &profile_with_habits());
        assert!(!result.within_bounds);
        assert_eq!(result.reason, Some("missing_greeting"));
    }

    #[test]
    fn missing_signoff_is_drift() {
        // Greets, but ends with no sign-off though the author always signs off.
        let body = "Hi Daniel,\n\nThe renewal terms are confirmed and we will proceed with the \
updated schedule for the next quarter as requested.";
        let result = check_style_drift(body, &profile_with_habits());
        assert!(!result.within_bounds);
        assert_eq!(result.reason, Some("missing_signoff"));
    }

    #[test]
    fn no_recorded_habit_never_forces_a_greeting_or_signoff() {
        // The same bare body passes when the profile records no greeting habit.
        let body = "The renewal terms are confirmed and we will proceed with the updated \
schedule.";
        assert!(check_style_drift(body, &profile_no_habits()).within_bounds);
    }

    #[test]
    fn all_caps_body_is_drift() {
        let body = "HI DANIEL, THE RENEWAL TERMS ARE CONFIRMED AND WE WILL PROCEED IMMEDIATELY. \
BEST REGARDS, MAYA";
        let result = check_style_drift(body, &profile_with_habits());
        assert!(!result.within_bounds);
        assert_eq!(result.reason, Some("all_caps"));
    }

    #[test]
    fn short_acronym_reply_is_not_mistaken_for_shouting() {
        // Under the all-caps length floor and otherwise on-style.
        let body = "Hi Tom,\n\nFYI the SOW and NDA are done.\n\nBest,\nMaya";
        assert!(check_style_drift(body, &profile_with_habits()).within_bounds);
    }

    #[test]
    fn empty_draft_is_drift() {
        assert_eq!(
            check_style_drift("   ", &profile_with_habits()).reason,
            Some("empty_draft")
        );
    }
}
