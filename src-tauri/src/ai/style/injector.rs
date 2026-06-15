//! Style-block construction for draft prompts (T076, F_E5 §4.4).
//!
//! [`build_style_block`] is a pure function: it renders the stored
//! six-dimension summary into the English style section that
//! `DraftPromptBuilder` (T079) inserts right after the system block. When the
//! profile is absent or incomplete it falls back to the role-based polite
//! template (AI_MODES §6.7 cold start) and reports `was_fallback = true` so
//! the E6 UI can show its "AI is still learning your style" badge.
//!
//! `sample_snippets` are deliberately never rendered (F_E5 §4.4), and callers
//! never log the returned text — only the fallback boolean (09 §5).

use super::profiler::StyleProfileJson;

/// Header line of the learned-style block (F_E5 §4.4 template).
pub const STYLE_BLOCK_HEADER: &str = "Please match the following personal writing style:";

/// Second line of the cold-start fallback (AI_MODES §6.7).
pub const COLD_START_NOTE: &str = "(Style calibration in progress — adapt to feedback over time.)";

/// At most this many opening/closing phrases are rendered, keeping the block
/// within its ~150-token budget regardless of how verbose the profile is.
pub const MAX_PATTERNS: usize = 3;

/// Render the style block for one account. Returns the block text plus
/// `was_fallback`: `true` when the cold-start template was used (no profile,
/// or an empty `overall_tone`).
pub fn build_style_block(profile: Option<&StyleProfileJson>, account_role: &str) -> (String, bool) {
    match profile {
        Some(p) if !p.summary.overall_tone.trim().is_empty() => {
            let s = &p.summary;
            let block = format!(
                "{STYLE_BLOCK_HEADER}\n\
                 - Tone: {}\n\
                 - Common openings: {}\n\
                 - Common closings: {}\n\
                 - Sentence length: {}\n\
                 - Format habits: {}",
                s.overall_tone.trim(),
                join_capped(&s.opening_patterns),
                join_capped(&s.closing_patterns),
                s.sentence_length.trim(),
                s.format_habit.trim(),
            );
            (block, false)
        }
        _ => {
            let block = format!(
                "Write in a professional and courteous tone consistent with the role: \
                 {account_role}.\n{COLD_START_NOTE}"
            );
            (block, true)
        }
    }
}

/// Join at most [`MAX_PATTERNS`] non-empty phrases with `", "`.
fn join_capped(patterns: &[String]) -> String {
    patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .take(MAX_PATTERNS)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::style::profiler::StyleSummary;
    use crate::ai::style::STYLE_PROFILE_VERSION;

    fn profile_with(summary: StyleSummary) -> StyleProfileJson {
        StyleProfileJson {
            version: STYLE_PROFILE_VERSION,
            account_id: "acc".into(),
            generated_at: 1_750_000_000,
            summary,
            sample_snippets: vec!["Hi Daniel, thanks for the revised SOW.".into()],
            pinned: false,
        }
    }

    fn full_summary() -> StyleSummary {
        StyleSummary {
            overall_tone: "Warm but direct; leads with the decision.".into(),
            opening_patterns: vec!["Hi {name},".into(), "Thanks for the update".into()],
            closing_patterns: vec!["Best regards,".into(), "Talk soon,".into()],
            sentence_length: "12-18 words on average".into(),
            vocabulary: "Plain business English".into(),
            format_habit: "Short paragraphs; bullets for action items.".into(),
        }
    }

    #[test]
    fn full_profile_renders_all_five_fields() {
        let profile = profile_with(full_summary());
        let (block, fallback) = build_style_block(Some(&profile), "Legal assistant");
        assert!(!fallback);
        assert!(block.starts_with(STYLE_BLOCK_HEADER));
        assert!(block.contains("Tone: Warm but direct; leads with the decision."));
        assert!(block.contains("Common openings: Hi {name},, Thanks for the update"));
        assert!(block.contains("Common closings: Best regards,, Talk soon,"));
        assert!(block.contains("Sentence length: 12-18 words on average"));
        assert!(block.contains("Format habits: Short paragraphs; bullets for action items."));
        // Snippets must never leak into the prompt (F_E5 §4.4).
        assert!(!block.contains("Hi Daniel"));
    }

    #[test]
    fn none_profile_uses_cold_start_template() {
        let (block, fallback) = build_style_block(None, "Work assistant");
        assert!(fallback);
        assert!(block.contains("professional and courteous"));
        assert!(block.contains("Work assistant"));
        assert!(block.contains(COLD_START_NOTE));
    }

    #[test]
    fn empty_overall_tone_falls_back() {
        let mut summary = full_summary();
        summary.overall_tone = "   ".into();
        let profile = profile_with(summary);
        let (block, fallback) = build_style_block(Some(&profile), "Sales assistant");
        assert!(fallback);
        assert!(block.contains("Sales assistant"));
        assert!(!block.contains(STYLE_BLOCK_HEADER));
    }

    #[test]
    fn long_pattern_lists_are_capped_at_three() {
        let mut summary = full_summary();
        summary.opening_patterns = (0..10).map(|i| format!("Opening {i}")).collect();
        let profile = profile_with(summary);
        let (block, fallback) = build_style_block(Some(&profile), "Work assistant");
        assert!(!fallback);
        assert!(block.contains("Opening 0, Opening 1, Opening 2"));
        assert!(!block.contains("Opening 3"));
    }

    #[test]
    fn blank_patterns_are_skipped_without_panicking() {
        let mut summary = full_summary();
        summary.opening_patterns = vec!["  ".into(), "Hello,".into()];
        summary.closing_patterns = Vec::new();
        let profile = profile_with(summary);
        let (block, fallback) = build_style_block(Some(&profile), "Work assistant");
        assert!(!fallback);
        assert!(block.contains("Common openings: Hello,"));
        assert!(block.contains("Common closings: \n"));
    }
}
