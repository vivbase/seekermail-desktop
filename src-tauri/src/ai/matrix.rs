//! F4 capability × account provider matrix (T065, F_F4 §4).
//!
//! One [`CapabilityMatrix`] per account persists in
//! `account_ai_settings.provider_matrix` as JSON (`NULL` = not configured).
//! Each [`MatrixEntry`] routes one [`Capability`] to a [`MatrixCell`]: a
//! primary [`ProviderAssignment`] plus an ordered backup chain of at most
//! [`MAX_BACKUPS`] links (F_F4 §6 — the cap keeps the T067 offline-fallback
//! walk bounded and prevents recursive cascades).
//!
//! [`AiRegistry::resolve`](super::registry::AiRegistry::resolve) consults the
//! matrix first; a capability with no entry falls back to the account's base
//! `ai_provider` / `ai_model` columns (the pre-T065 behavior).
//!
//! Saving a matrix also produces advisory [`MatrixWarning`]s (F_F4 §4.5):
//! they flag likely cost/capability/privacy mismatches but never block the
//! user's choice.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};
use crate::types::AiProvider;

use super::registry::AccountAiConfig;
use super::types::Capability;

/// Hard cap on the backup-chain length per cell (F_F4 §6).
pub const MAX_BACKUPS: usize = 2;

/// All routable capabilities, in the row order the matrix UI presents them
/// (T066). The feature mapping is: `DraftReply` ⇒ E1/E2/E3, `RiskReason` ⇒ E4,
/// `Summarize` ⇒ B3/D1/D2, `StyleProfile` ⇒ E5 (T065 §6).
pub const ALL_CAPABILITIES: [Capability; 4] = [
    Capability::DraftReply,
    Capability::RiskReason,
    Capability::Summarize,
    Capability::StyleProfile,
];

/// One provider + model choice inside a cell (F_F4 §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAssignment {
    pub provider: AiProvider,
    /// Model name at that provider. An empty string means "the provider's
    /// default" (the local ONNX adapter auto-selects from its models folder).
    pub model: String,
    /// Custom endpoint override; `None` = the provider's standard endpoint
    /// (or, when the assignment keeps the account's base provider, the base
    /// `ai_base_url`).
    pub base_url: Option<String>,
}

/// The assignment for one `(capability, account)` cell: a required primary
/// plus an ordered fallback chain (F_F4 §4.2). `resolve()` only ever returns
/// the primary; T067 walks `backups` after a primary failure via
/// [`AiRegistry::resolve_backup`](super::registry::AiRegistry::resolve_backup).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MatrixCell {
    pub primary: ProviderAssignment,
    #[serde(default)]
    pub backups: Vec<ProviderAssignment>,
}

impl MatrixCell {
    /// Structural checks for one cell (F_F4 §4.2, §6): at most [`MAX_BACKUPS`]
    /// backups, and no backup may repeat the primary's provider.
    pub fn validate(&self) -> AppResult<()> {
        if self.backups.len() > MAX_BACKUPS {
            return Err(AppError::Validation(format!(
                "a matrix cell allows at most {MAX_BACKUPS} backup assignments"
            )));
        }
        for backup in &self.backups {
            if backup.provider == self.primary.provider {
                return Err(AppError::Validation(format!(
                    "backup provider '{}' duplicates the cell's primary provider",
                    backup.provider.as_str()
                )));
            }
        }
        Ok(())
    }
}

/// One matrix row binding for an account: capability → cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MatrixEntry {
    pub capability: Capability,
    pub cell: MatrixCell,
}

/// The full per-account matrix (F_F4 §4.4), persisted as JSON in
/// `account_ai_settings.provider_matrix`. An empty `entries` list is valid —
/// every capability then falls back to the base provider columns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityMatrix {
    pub entries: Vec<MatrixEntry>,
}

impl CapabilityMatrix {
    /// Parse the persisted JSON column. Unreadable payloads surface as
    /// `VALIDATION`; callers on the routing path degrade to "no matrix"
    /// instead of failing the AI call.
    pub fn from_json(s: &str) -> AppResult<Self> {
        serde_json::from_str(s)
            .map_err(|e| AppError::Validation(format!("provider matrix json is invalid: {e}")))
    }

    /// Serialize for the `provider_matrix` column. Infallible for these plain
    /// data types.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("capability matrix serializes to json")
    }

    /// The cell routing `cap`, when the matrix has one.
    pub fn cell(&self, cap: Capability) -> Option<&MatrixCell> {
        self.entries
            .iter()
            .find(|e| e.capability == cap)
            .map(|e| &e.cell)
    }

    /// Insert or replace the cell for `cap`, preserving entry order.
    pub fn set_cell(&mut self, cap: Capability, cell: MatrixCell) {
        match self.entries.iter_mut().find(|e| e.capability == cap) {
            Some(entry) => entry.cell = cell,
            None => self.entries.push(MatrixEntry {
                capability: cap,
                cell,
            }),
        }
    }

    /// Whole-matrix structural validation (T065 §3): every cell passes
    /// [`MatrixCell::validate`] and no capability appears twice (a duplicate
    /// row would make routing ambiguous).
    pub fn validate(&self) -> AppResult<()> {
        for (i, entry) in self.entries.iter().enumerate() {
            entry.cell.validate()?;
            if self.entries[..i]
                .iter()
                .any(|prior| prior.capability == entry.capability)
            {
                return Err(AppError::Validation(format!(
                    "capability '{}' is assigned more than once",
                    entry.capability.as_str()
                )));
            }
        }
        Ok(())
    }

    /// Advisory checks run on save (F_F4 §4.5). Warnings never block the
    /// write — `update_provider_matrix` persists first and returns these for
    /// the UI's non-blocking yellow hints.
    pub fn warnings(&self) -> Vec<MatrixWarning> {
        self.entries.iter().filter_map(warning_for).collect()
    }
}

/// The advisory warning for one entry, when its primary assignment looks like
/// a capability/cost/privacy mismatch (F_F4 §4.5). Always `None` for
/// `DraftReply` — drafting has no spec-defined hint.
fn warning_for(entry: &MatrixEntry) -> Option<MatrixWarning> {
    let primary = &entry.cell.primary;
    match entry.capability {
        Capability::Summarize => {
            // Covers B3/D1/D2: role audits need strong reasoning + long context.
            let local = matches!(primary.provider, AiProvider::LocalOnnx | AiProvider::Ollama);
            (local && is_small_local_model(&primary.model)).then(|| MatrixWarning {
                capability: entry.capability,
                code: "small_local_model".into(),
                message: format!(
                    "Summaries and role audits need strong reasoning; '{}' looks smaller than 7B and may underperform.",
                    primary.model
                ),
            })
        }
        Capability::RiskReason => primary.provider.is_cloud().then(|| MatrixWarning {
            capability: entry.capability,
            code: "high_cost_cloud".into(),
            message: String::from(
                "Sensitivity checks run on every inbound mail; a cloud model here can add significant cost.",
            ),
        }),
        Capability::StyleProfile => primary.provider.is_cloud().then(|| MatrixWarning {
            capability: entry.capability,
            code: "style_history_to_cloud".into(),
            message: String::from(
                "Style learning sends excerpts of your mail history to a public cloud endpoint; consider a local model.",
            ),
        }),
        Capability::DraftReply => None,
    }
}

/// One non-blocking save-time hint (F_F4 §4.5). `code` is a stable tag the
/// frontend maps to localized copy; `message` is the English fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MatrixWarning {
    pub capability: Capability,
    pub code: String,
    pub message: String,
}

/// One item of a `batch_update_provider_matrix` call (T066 batch operations:
/// copy row / copy column / switch-all-E4-to-local, F_F4 §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchMatrixUpdate {
    pub account_id: String,
    pub capability: Capability,
    pub cell: MatrixCell,
}

/// Compute the account's default matrix (F_F4 §4.1) without persisting it:
///
/// * Every capability defaults to the account's base provider/model.
/// * When a `local_onnx` adapter is registered, `RiskReason` (E4) and
///   `StyleProfile` (E5) prefer the local model instead.
/// * A capability whose computed primary would be `none` gets no entry, so an
///   account with no provider at all yields an empty matrix (the UI then
///   routes the user to F1/F3 onboarding).
/// * `backups` start empty.
///
/// Deterministic for a given `(base, registered)` input, so re-running a
/// reset writes the same matrix (T065 §6 idempotency).
pub fn build_default_matrix(base: &AccountAiConfig, registered: &[AiProvider]) -> CapabilityMatrix {
    let local_available = registered.contains(&AiProvider::LocalOnnx);
    let base_assignment = (base.provider != AiProvider::None).then(|| ProviderAssignment {
        provider: base.provider,
        model: base.model.clone().unwrap_or_default(),
        base_url: base.base_url.clone(),
    });
    let local_assignment = ProviderAssignment {
        provider: AiProvider::LocalOnnx,
        // Empty = let the local adapter auto-select from its models folder.
        model: String::new(),
        base_url: None,
    };

    let mut entries = Vec::new();
    for cap in ALL_CAPABILITIES {
        let prefers_local = matches!(cap, Capability::RiskReason | Capability::StyleProfile);
        let primary = if local_available && prefers_local {
            Some(local_assignment.clone())
        } else {
            base_assignment.clone()
        };
        if let Some(primary) = primary {
            entries.push(MatrixEntry {
                capability: cap,
                cell: MatrixCell {
                    primary,
                    backups: Vec::new(),
                },
            });
        }
    }
    CapabilityMatrix { entries }
}

/// Heuristic "< 7B parameters" detection from the model name (F_F4 §4.5):
/// finds a number directly followed by `b` at a word boundary (`1.5b`, `3B`,
/// `gemma-2b-it`) and compares it to 7. Names without a parameter-count tag
/// are not flagged.
fn is_small_local_model(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'b' {
                let boundary = bytes
                    .get(i + 1)
                    .map_or(true, |c| !c.is_ascii_alphanumeric());
                if boundary {
                    if let Ok(size) = lower[start..i].parse::<f32>() {
                        return size < 7.0;
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ErrorCode;

    fn assignment(provider: AiProvider, model: &str) -> ProviderAssignment {
        ProviderAssignment {
            provider,
            model: model.into(),
            base_url: None,
        }
    }

    fn base_config(provider: AiProvider, model: Option<&str>) -> AccountAiConfig {
        AccountAiConfig {
            account_id: "acct-1".into(),
            provider,
            model: model.map(str::to_string),
            base_url: None,
            api_key_ref: None,
            daily_query_limit: 10,
            updated_at: 1,
        }
    }

    #[test]
    fn matrix_serde_roundtrip() {
        let matrix = CapabilityMatrix {
            entries: vec![MatrixEntry {
                capability: Capability::DraftReply,
                cell: MatrixCell {
                    primary: ProviderAssignment {
                        provider: AiProvider::Anthropic,
                        model: "claude-sonnet-4-5".into(),
                        base_url: Some("https://gateway.example.com".into()),
                    },
                    backups: vec![assignment(AiProvider::Openai, "gpt-4o")],
                },
            }],
        };
        let json = matrix.to_json();
        // Wire spellings: PascalCase capability, snake_case provider (T065 §6).
        assert!(json.contains("\"DraftReply\""));
        assert!(json.contains("\"anthropic\""));
        assert!(json.contains("\"baseUrl\""));
        let parsed = CapabilityMatrix::from_json(&json).unwrap();
        assert_eq!(parsed, matrix);
    }

    #[test]
    fn from_json_rejects_garbage_and_defaults_missing_backups() {
        let err = CapabilityMatrix::from_json("{not json").unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);

        // `backups` is optional on the wire (serde default).
        let json = r#"{"entries":[{"capability":"Summarize","cell":{"primary":{"provider":"ollama","model":"llama3.1-8b","baseUrl":null}}}]}"#;
        let parsed = CapabilityMatrix::from_json(json).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert!(parsed.entries[0].cell.backups.is_empty());
    }

    #[test]
    fn validate_rejects_backup_chain_longer_than_two() {
        let matrix = CapabilityMatrix {
            entries: vec![MatrixEntry {
                capability: Capability::DraftReply,
                cell: MatrixCell {
                    primary: assignment(AiProvider::Anthropic, "claude-sonnet-4-5"),
                    backups: vec![
                        assignment(AiProvider::Openai, "gpt-4o"),
                        assignment(AiProvider::Ollama, "llama3.1-8b"),
                        assignment(AiProvider::LocalOnnx, ""),
                    ],
                },
            }],
        };
        let err = matrix.validate().unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[test]
    fn validate_rejects_backup_repeating_primary_provider() {
        let matrix = CapabilityMatrix {
            entries: vec![MatrixEntry {
                capability: Capability::RiskReason,
                cell: MatrixCell {
                    primary: assignment(AiProvider::Openai, "gpt-4o"),
                    backups: vec![assignment(AiProvider::Openai, "gpt-4o-mini")],
                },
            }],
        };
        let err = matrix.validate().unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[test]
    fn validate_rejects_duplicate_capability_and_accepts_a_full_cell() {
        let cell = MatrixCell {
            primary: assignment(AiProvider::Anthropic, "claude-sonnet-4-5"),
            backups: vec![
                assignment(AiProvider::Openai, "gpt-4o"),
                assignment(AiProvider::Ollama, "llama3.1-8b"),
            ],
        };
        let valid = CapabilityMatrix {
            entries: vec![MatrixEntry {
                capability: Capability::DraftReply,
                cell: cell.clone(),
            }],
        };
        valid.validate().unwrap();

        let duplicated = CapabilityMatrix {
            entries: vec![
                MatrixEntry {
                    capability: Capability::DraftReply,
                    cell: cell.clone(),
                },
                MatrixEntry {
                    capability: Capability::DraftReply,
                    cell,
                },
            ],
        };
        let err = duplicated.validate().unwrap_err();
        assert_eq!(err.code(), ErrorCode::Validation);
    }

    #[test]
    fn default_matrix_prefers_local_onnx_for_risk_and_style() {
        let base = base_config(AiProvider::Openai, Some("gpt-4o"));
        let registered = [AiProvider::LocalOnnx, AiProvider::Openai];
        let matrix = build_default_matrix(&base, &registered);

        assert_eq!(
            matrix
                .cell(Capability::RiskReason)
                .unwrap()
                .primary
                .provider,
            AiProvider::LocalOnnx
        );
        assert_eq!(
            matrix
                .cell(Capability::StyleProfile)
                .unwrap()
                .primary
                .provider,
            AiProvider::LocalOnnx
        );
        assert_eq!(
            matrix
                .cell(Capability::DraftReply)
                .unwrap()
                .primary
                .provider,
            AiProvider::Openai
        );
        assert_eq!(
            matrix.cell(Capability::Summarize).unwrap().primary.model,
            "gpt-4o"
        );
        // Backups start empty; result is deterministic (idempotent reset).
        assert!(matrix.entries.iter().all(|e| e.cell.backups.is_empty()));
        assert_eq!(matrix, build_default_matrix(&base, &registered));
    }

    #[test]
    fn default_matrix_without_local_uses_base_everywhere() {
        let base = base_config(AiProvider::Anthropic, Some("claude-sonnet-4-5"));
        let matrix = build_default_matrix(&base, &[AiProvider::Anthropic]);
        assert_eq!(matrix.entries.len(), ALL_CAPABILITIES.len());
        assert!(matrix
            .entries
            .iter()
            .all(|e| e.cell.primary.provider == AiProvider::Anthropic));
    }

    #[test]
    fn default_matrix_with_no_provider_is_empty() {
        let base = base_config(AiProvider::None, None);
        let matrix = build_default_matrix(&base, &[]);
        assert!(matrix.entries.is_empty());
    }

    #[test]
    fn warnings_flag_cloud_risk_and_style_and_small_local_summarize() {
        let matrix = CapabilityMatrix {
            entries: vec![
                MatrixEntry {
                    capability: Capability::RiskReason,
                    cell: MatrixCell {
                        primary: assignment(AiProvider::Anthropic, "claude-opus-4-1"),
                        backups: Vec::new(),
                    },
                },
                MatrixEntry {
                    capability: Capability::StyleProfile,
                    cell: MatrixCell {
                        primary: assignment(AiProvider::Openai, "gpt-4o"),
                        backups: Vec::new(),
                    },
                },
                MatrixEntry {
                    capability: Capability::Summarize,
                    cell: MatrixCell {
                        primary: assignment(AiProvider::Ollama, "gemma-2b-it"),
                        backups: Vec::new(),
                    },
                },
            ],
        };
        let warnings = matrix.warnings();
        let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();
        assert!(codes.contains(&"high_cost_cloud"));
        assert!(codes.contains(&"style_history_to_cloud"));
        assert!(codes.contains(&"small_local_model"));
        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn warnings_stay_quiet_for_sensible_assignments() {
        let matrix = CapabilityMatrix {
            entries: vec![
                MatrixEntry {
                    capability: Capability::RiskReason,
                    cell: MatrixCell {
                        primary: assignment(AiProvider::LocalOnnx, ""),
                        backups: Vec::new(),
                    },
                },
                MatrixEntry {
                    capability: Capability::Summarize,
                    cell: MatrixCell {
                        primary: assignment(AiProvider::Ollama, "llama3.1-70b"),
                        backups: Vec::new(),
                    },
                },
                MatrixEntry {
                    capability: Capability::DraftReply,
                    cell: MatrixCell {
                        primary: assignment(AiProvider::Openai, "gpt-4o"),
                        backups: Vec::new(),
                    },
                },
            ],
        };
        assert!(matrix.warnings().is_empty());
    }

    #[test]
    fn small_model_heuristic_handles_common_name_shapes() {
        assert!(is_small_local_model("gemma-2b-it"));
        assert!(is_small_local_model("qwen2-1.5B-instruct"));
        assert!(is_small_local_model("phi-3.5b"));
        assert!(!is_small_local_model("llama3.1-8b"));
        assert!(!is_small_local_model("llama-3.1-70b"));
        assert!(!is_small_local_model("mixtral-8x22b")); // first b-tag is 22b
        assert!(!is_small_local_model("")); // no tag → not flagged
        assert!(!is_small_local_model("claude-sonnet-4-5"));
    }
}
