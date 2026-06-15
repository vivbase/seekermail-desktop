//! E3 style-drift validation hook (T076 §3 — interface only).
//!
//! [`check_style_drift`] is the public seam T085 (E3 full-auto send) fills
//! with the real AI-scored comparison. Until then it always reports
//! `within_bounds: true`, so no caller is ever blocked by an unimplemented
//! check while the wiring (call site, result shape) lands now.

use super::profiler::StyleProfileJson;

/// Outcome of a style-drift check on a generated draft (T085 consumes it
/// before an autonomous send).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleDriftResult {
    /// `true` — the draft is close enough to the learned style to auto-send.
    pub within_bounds: bool,
}

/// Compare a generated draft against the learned profile. v0.7 behaviour:
/// always within bounds; the scoring model arrives with T085.
pub fn check_style_drift(_draft_text: &str, _profile: &StyleProfileJson) -> StyleDriftResult {
    StyleDriftResult {
        within_bounds: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::style::profiler::StyleSummary;
    use crate::ai::style::STYLE_PROFILE_VERSION;

    #[test]
    fn stub_always_reports_within_bounds() {
        let profile = StyleProfileJson {
            version: STYLE_PROFILE_VERSION,
            account_id: "acc".into(),
            generated_at: 1_750_000_000,
            summary: StyleSummary {
                overall_tone: "Concise and courteous.".into(),
                opening_patterns: vec!["Hi,".into()],
                closing_patterns: vec!["Best,".into()],
                sentence_length: "short".into(),
                vocabulary: "plain".into(),
                format_habit: "short paragraphs".into(),
            },
            sample_snippets: Vec::new(),
            pinned: false,
        };
        assert!(check_style_drift("", &profile).within_bounds);
        assert!(check_style_drift("Any draft text at all.", &profile).within_bounds);
    }
}
