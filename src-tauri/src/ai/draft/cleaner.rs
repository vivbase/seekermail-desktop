//! Draft body post-processing (T077 §3, F_E1 §4.3). Pure functions shared by
//! every E-mode: the provider's raw completion is normalized to plain text
//! before it reaches `ai_drafts.body_original` and the Tiptap editor.
//!
//! Two passes:
//! 1. **Fence stripping** — models occasionally wrap the reply in markdown
//!    code fences; every fence-marker line is removed.
//! 2. **Signature dedup** — when the final paragraph overlaps the caller's
//!    `signature_hint` beyond a Jaccard similarity of 0.8, it is dropped so
//!    the account's own signature is never doubled (F_E1 §4.3).

use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

/// Final-paragraph word overlap above which the tail counts as a duplicated
/// signature (T077 §3).
const SIGNATURE_JACCARD_THRESHOLD: f64 = 0.8;

/// A whole line that is a markdown fence marker — opening (with an optional
/// language tag) or closing — including its trailing newline.
static FENCE_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^[ \t]*```[A-Za-z0-9_+-]*[ \t]*\r?\n?").expect("fence regex is valid")
});

/// Normalize one raw AI completion into plain draft text. `signature_hint`
/// (typically the account's role/signature text) drives the tail dedup; pass
/// `None` to skip it.
pub fn clean_ai_body(raw: &str, signature_hint: Option<&str>) -> String {
    let without_fences = FENCE_LINE.replace_all(raw, "");
    let mut text = without_fences.trim().to_string();

    if let Some(hint) = signature_hint.map(str::trim).filter(|h| !h.is_empty()) {
        if let Some(tail_start) = tail_block_start(&text) {
            let tail = &text[tail_start..];
            if jaccard_similarity(tail, hint) > SIGNATURE_JACCARD_THRESHOLD {
                text = text[..tail_start].trim_end().to_string();
            }
        }
    }
    text
}

/// Byte offset where the final paragraph (after the last blank line) starts.
/// `None` when the text is a single block — a lone paragraph is never treated
/// as a removable signature.
fn tail_block_start(text: &str) -> Option<usize> {
    text.rfind("\n\n").map(|i| i + 2)
}

/// Case-insensitive word-set Jaccard similarity; 0.0 when either side is empty.
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<String> = a.split_whitespace().map(str::to_lowercase).collect();
    let set_b: HashSet<String> = b.split_whitespace().map(str::to_lowercase).collect();
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    intersection as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_block_with_language_tag_is_unwrapped() {
        let raw = "```python\nDear Daniel,\n\nThe revised terms look fine.\n```";
        let cleaned = clean_ai_body(raw, None);
        assert!(!cleaned.contains("```"));
        assert!(cleaned.starts_with("Dear Daniel,"));
        assert!(cleaned.ends_with("The revised terms look fine."));
    }

    #[test]
    fn plain_fences_anywhere_are_removed() {
        let raw = "Intro line\n```\nquoted part\n```\nClosing line";
        let cleaned = clean_ai_body(raw, None);
        assert_eq!(cleaned, "Intro line\nquoted part\nClosing line");
    }

    #[test]
    fn identical_trailing_signature_is_deduped() {
        let raw = "Thanks for the update — the timeline works for us.\n\nBest regards,\nMaya Chen";
        let cleaned = clean_ai_body(raw, Some("Best regards,\nMaya Chen"));
        assert_eq!(
            cleaned,
            "Thanks for the update — the timeline works for us."
        );
    }

    #[test]
    fn dissimilar_tail_paragraph_is_kept() {
        let raw = "Thanks for the update.\n\nCould you send the revised schedule by Friday?";
        let cleaned = clean_ai_body(raw, Some("Best regards,\nMaya Chen"));
        assert_eq!(cleaned, raw);
    }

    #[test]
    fn clean_text_passes_through_unchanged() {
        let raw = "Hi Daniel,\n\nThe contract looks good to me.\n\nCould we sign on Monday?";
        assert_eq!(clean_ai_body(raw, None), raw);
        assert_eq!(clean_ai_body(raw, Some("")), raw);
    }

    #[test]
    fn single_paragraph_is_never_stripped_as_a_signature() {
        let raw = "Best regards, Maya Chen";
        let cleaned = clean_ai_body(raw, Some("Best regards, Maya Chen"));
        assert_eq!(
            cleaned, raw,
            "a lone paragraph is the whole draft, not a signature"
        );
    }

    #[test]
    fn jaccard_basics() {
        assert!(jaccard_similarity("best regards maya", "Best Regards Maya") > 0.99);
        assert_eq!(jaccard_similarity("", "anything"), 0.0);
        assert!(jaccard_similarity("alpha beta", "gamma delta") < f64::EPSILON);
    }
}
