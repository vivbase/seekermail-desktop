//! Module D commands — role-based mail analysis (T070 legal / F_D1,
//! T072 sales / F_D2).
//!
//! Thin wrappers per the command-layer convention (03 §1): deserialize params,
//! call exactly one pipeline, map `AppError → IpcError`. The whole D1 pipeline
//! — 24-hour cache, T074 context assembly, provider call, strict JSON
//! validation with one retry, audit + risk-event persistence — lives in
//! [`crate::ai::legal`]; its D2 sibling (same shape, contact history, no
//! `risk_events`) lives in [`crate::ai::sales`].

use tauri::State;

use crate::ai::legal::LegalAnalysisPipeline;
use crate::ai::sales::SalesAnalysisPipeline;
use crate::error::IpcError;
use crate::state::AppState;
use crate::types::{
    AnalyzeLegalRiskParams, AnalyzeSalesContextParams, LegalAnalysisResult, SalesAnalysisResult,
};

/// Run (or replay the cached) D1 legal risk analysis for one mail (T070 §3).
///
/// `forceNew = false` returns the most recent analysis within 24 hours without
/// touching the AI provider (F_D1 §4.5). Errors: `NOT_FOUND` (unknown mail or
/// missing AI settings row), `FORBIDDEN` (AI not configured for the account),
/// `AI_RATE_LIMITED` (daily query limit), `AI_PROVIDER_UNREACHABLE`,
/// `AI_CONTEXT_TOO_LONG`, `INTERNAL` (model output invalid after one retry —
/// the frontend degrades to plain-text display, F_D1 §6).
#[tauri::command]
pub async fn analyze_legal_risk(
    state: State<'_, AppState>,
    params: AnalyzeLegalRiskParams,
) -> Result<LegalAnalysisResult, IpcError> {
    LegalAnalysisPipeline::new(&state)
        .run(&params)
        .await
        .map_err(IpcError::from)
}

/// Run (or replay the cached) D2 sales / negotiation analysis for one mail
/// (T072 §3).
///
/// `forceNew = false` returns the most recent analysis within 24 hours without
/// touching the AI provider (T072 §3 step 1). D2 writes one `ai_decisions`
/// audit row and never writes `risk_events` (business judgement is context
/// assistance, not a safety risk). Errors: `NOT_FOUND` (unknown mail or
/// missing AI settings row), `FORBIDDEN` (AI not configured for the account),
/// `AI_RATE_LIMITED` (daily query limit), `AI_PROVIDER_UNREACHABLE`,
/// `AI_CONTEXT_TOO_LONG`, `INTERNAL` (model output invalid after one retry).
#[tauri::command]
pub async fn analyze_sales_context(
    state: State<'_, AppState>,
    params: AnalyzeSalesContextParams,
) -> Result<SalesAnalysisResult, IpcError> {
    SalesAnalysisPipeline::new(&state)
        .run(&params)
        .await
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use crate::types::{
        AnalyzeLegalRiskParams, AnalyzeSalesContextParams, ConcessionAdvice, ContactHistorySummary,
        CounterpartyProfile, CounterpartyStance, CounterpartyTone, LegalAnalysisResult,
        LegalKeyClauses, LegalOverallLevel, LegalRiskItem, LegalRiskLevel, LegalRiskType, NeedItem,
        NeedPriority, NextAction, NextActionTimeline, SalesAnalysisResult,
    };

    /// `forceNew` is optional on the wire and defaults to the cached path.
    #[test]
    fn params_default_force_new_to_false() {
        let params: AnalyzeLegalRiskParams =
            serde_json::from_str(r#"{"mailId":"5f2d6a1e-0000-4000-8000-000000000001"}"#).unwrap();
        assert!(!params.force_new);

        let params: AnalyzeLegalRiskParams = serde_json::from_str(
            r#"{"mailId":"5f2d6a1e-0000-4000-8000-000000000001","forceNew":true}"#,
        )
        .unwrap();
        assert!(params.force_new);
    }

    /// The wire shape matches the card's IPC contract: camelCase keys, the
    /// risk item's category serialized as `type`, lowercase level tags.
    #[test]
    fn result_serializes_to_card_wire_shape() {
        let result = LegalAnalysisResult {
            decision_id: "d1".into(),
            mail_id: "m1".into(),
            account_id: "a1".into(),
            risk_list: vec![LegalRiskItem {
                level: LegalRiskLevel::High,
                risk_type: LegalRiskType::Payment,
                original_text: "Payment due within 90 days".into(),
                finding: "Unusually long payment term".into(),
                suggestion: "Negotiate net-30 terms".into(),
            }],
            key_clauses: LegalKeyClauses {
                payment: Some("Net 90".into()),
                ..LegalKeyClauses::default()
            },
            compliance_advice: vec!["Shorten payment terms".into()],
            overall_level: LegalOverallLevel::High,
            ai_model: "gpt-test".into(),
            knowledge_refs: vec!["k1".into()],
            created_at: 1,
        };
        let wire: serde_json::Value = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["decisionId"], "d1");
        assert_eq!(wire["overallLevel"], "high");
        assert_eq!(wire["riskList"][0]["type"], "payment");
        assert_eq!(wire["riskList"][0]["level"], "high");
        assert_eq!(
            wire["riskList"][0]["originalText"],
            "Payment due within 90 days"
        );
        assert_eq!(wire["keyClauses"]["payment"], "Net 90");
        assert_eq!(wire["knowledgeRefs"][0], "k1");
        // Round-trip: the same JSON backs the 24-hour cache replay.
        let back: LegalAnalysisResult = serde_json::from_value(wire).unwrap();
        assert_eq!(back, result);
    }

    /// `forceNew` is optional on the D2 wire as well, defaulting to cached.
    #[test]
    fn sales_params_default_force_new_to_false() {
        let params: AnalyzeSalesContextParams =
            serde_json::from_str(r#"{"mailId":"5f2d6a1e-0000-4000-8000-000000000002"}"#).unwrap();
        assert!(!params.force_new);

        let params: AnalyzeSalesContextParams = serde_json::from_str(
            r#"{"mailId":"5f2d6a1e-0000-4000-8000-000000000002","forceNew":true}"#,
        )
        .unwrap();
        assert!(params.force_new);
    }

    /// The D2 wire shape matches the card's IPC contract: camelCase keys,
    /// lowercase stance/tone/priority tags, the schema-literal timeline tags
    /// (`24h` / `this_week`), and `contactHistory: null` on first contact.
    #[test]
    fn sales_result_serializes_to_card_wire_shape() {
        let result = SalesAnalysisResult {
            decision_id: "d2".into(),
            mail_id: "m1".into(),
            account_id: "a1".into(),
            counterparty_profile: CounterpartyProfile {
                stance: CounterpartyStance::Adversarial,
                tone: CounterpartyTone::Casual,
                authority_signal: "Defers to a steering committee".into(),
                observations: vec!["Pushes back on every price point".into()],
            },
            needs_and_intents: vec![NeedItem {
                need: "Lower unit price".into(),
                priority: NeedPriority::High,
                evidence: "your quote is well above our budget".into(),
            }],
            concession_advice: ConcessionAdvice {
                concedable: vec!["Extended warranty".into()],
                negotiable: vec!["Payment schedule".into()],
                non_concedable: vec!["Unit price below cost floor".into()],
            },
            next_actions: vec![
                NextAction {
                    action: "Send a revised quote".into(),
                    timeline: NextActionTimeline::Within24h,
                },
                NextAction {
                    action: "Schedule a follow-up call".into(),
                    timeline: NextActionTimeline::ThisWeek,
                },
            ],
            contact_history: None,
            ai_model: "gpt-test".into(),
            knowledge_refs: vec!["k1".into()],
            created_at: 1,
        };
        let wire: serde_json::Value = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["decisionId"], "d2");
        assert_eq!(wire["counterpartyProfile"]["stance"], "adversarial");
        assert_eq!(wire["counterpartyProfile"]["tone"], "casual");
        assert_eq!(
            wire["counterpartyProfile"]["authoritySignal"],
            "Defers to a steering committee"
        );
        assert_eq!(wire["needsAndIntents"][0]["priority"], "high");
        assert_eq!(
            wire["concessionAdvice"]["nonConcedable"][0],
            "Unit price below cost floor"
        );
        assert_eq!(wire["nextActions"][0]["timeline"], "24h");
        assert_eq!(wire["nextActions"][1]["timeline"], "this_week");
        assert_eq!(wire["contactHistory"], serde_json::Value::Null);
        assert_eq!(wire["knowledgeRefs"][0], "k1");
        // Round-trip: the same JSON backs the 24-hour cache replay.
        let back: SalesAnalysisResult = serde_json::from_value(wire).unwrap();
        assert_eq!(back, result);

        // Populated contact history serializes camelCase with parsed notes.
        let history = ContactHistorySummary {
            interaction_count: 17,
            reply_count: 9,
            style_notes: Some(serde_json::json!({"greeting": "Hi team"})),
        };
        let wire = serde_json::to_value(&history).unwrap();
        assert_eq!(wire["interactionCount"], 17);
        assert_eq!(wire["replyCount"], 9);
        assert_eq!(wire["styleNotes"]["greeting"], "Hi team");
    }
}
