//! `ai_decisions` repository — the single INSERT statement plus the E7 query,
//! aggregation, and export surface (T088, F_E7 §4).
//!
//! Append-only by API: this module exposes INSERT and SELECT only. The one
//! exception is the retention policy sweep (`retention.rs`), which DELETEs
//! rows past the configured age — a documented policy purge, not an edit path.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::{map_sqlx_err, Db};
use crate::util::{new_uuid, now_unix, truncate_chars};

use super::logger::{AuditEntry, DESCRIPTION_MAX_CHARS};
use super::types::{
    AiDecisionExportFormat, AiDecisionRow, DecisionSummary, ExportAiDecisionsParams,
    ListDecisionsParams,
};

/// Default and hard cap for `list_decisions` page size (F_E7 §7).
pub const LIST_DECISIONS_MAX_LIMIT: i64 = 200;

/// The ONE `ai_decisions` INSERT in the codebase (T088 §3). Engine
/// transactions, the fire-and-forget logger, and `log_await` all route here.
const INSERT_DECISION_SQL: &str = "INSERT INTO ai_decisions (id, account_id, mail_id, draft_id, \
     decision_type, impact, action_description, knowledge_refs, knowledge_summary, \
     result_description, ai_model, input_tokens, output_tokens, latency_ms, created_at) \
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

/// Bind + execute the shared INSERT against any SQLite executor. Descriptions
/// are truncated to [`DESCRIPTION_MAX_CHARS`] here so no caller can exceed the
/// cap. Returns the new row id.
async fn insert_with<'e, E>(executor: E, entry: &AuditEntry) -> AppResult<String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = new_uuid();
    let knowledge_refs_json =
        serde_json::to_string(&entry.knowledge_refs).unwrap_or_else(|_| "[]".into());
    sqlx::query(INSERT_DECISION_SQL)
        .bind(&id)
        .bind(&entry.account_id)
        .bind(&entry.mail_id)
        .bind(&entry.draft_id)
        .bind(&entry.decision_type)
        .bind(&entry.impact)
        .bind(truncate_chars(
            &entry.action_description,
            DESCRIPTION_MAX_CHARS,
        ))
        .bind(&knowledge_refs_json)
        .bind(&entry.knowledge_summary)
        .bind(truncate_chars(
            &entry.result_description,
            DESCRIPTION_MAX_CHARS,
        ))
        .bind(&entry.ai_model)
        .bind(entry.input_tokens)
        .bind(entry.output_tokens)
        .bind(entry.latency_ms)
        .bind(now_unix())
        .execute(executor)
        .await
        .map_err(map_sqlx_err)?;
    Ok(id)
}

/// Insert one audit row on the pool (the logger's path).
pub async fn insert_decision(db: &Db, entry: &AuditEntry) -> AppResult<String> {
    insert_with(db.pool(), entry).await
}

/// Insert one audit row inside an open transaction (the draft engine /
/// approve-draft path, where the decision must commit with the draft write).
pub async fn insert_decision_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    entry: &AuditEntry,
) -> AppResult<String> {
    insert_with(&mut **tx, entry).await
}

// ── Query & aggregation ───────────────────────────────────────────────────────

/// DB projection of one joined row (`knowledge_refs` still JSON text).
#[derive(sqlx::FromRow)]
struct DecisionDbRow {
    id: String,
    account_id: String,
    mail_id: Option<String>,
    draft_id: Option<String>,
    decision_type: String,
    impact: String,
    action_description: String,
    result_description: String,
    knowledge_refs: String,
    knowledge_summary: Option<String>,
    ai_model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    latency_ms: Option<i64>,
    created_at: i64,
    mail_subject: Option<String>,
}

impl From<DecisionDbRow> for AiDecisionRow {
    fn from(r: DecisionDbRow) -> Self {
        AiDecisionRow {
            id: r.id,
            account_id: r.account_id,
            mail_id: r.mail_id,
            draft_id: r.draft_id,
            decision_type: r.decision_type,
            impact: r.impact,
            action_description: r.action_description,
            result_description: r.result_description,
            knowledge_refs: serde_json::from_str(&r.knowledge_refs).unwrap_or_default(),
            knowledge_summary: r.knowledge_summary,
            ai_model: r.ai_model,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            latency_ms: r.latency_ms,
            created_at: r.created_at,
            mail_subject: r.mail_subject,
        }
    }
}

const DECISION_SELECT: &str = "SELECT d.id, d.account_id, d.mail_id, d.draft_id, \
     d.decision_type, d.impact, d.action_description, d.result_description, \
     d.knowledge_refs, d.knowledge_summary, d.ai_model, d.input_tokens, \
     d.output_tokens, d.latency_ms, d.created_at, m.subject AS mail_subject \
     FROM ai_decisions d LEFT JOIN mails m ON m.id = d.mail_id WHERE 1 = 1";

/// Shared filtered fetch. `limit = None` means unbounded (export path only).
#[allow(clippy::too_many_arguments)]
async fn fetch_decisions(
    db: &Db,
    account_id: Option<&str>,
    since: Option<i64>,
    until: Option<i64>,
    decision_types: Option<&[String]>,
    impact: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> AppResult<Vec<AiDecisionRow>> {
    let mut sql = String::from(DECISION_SELECT);
    if account_id.is_some() {
        sql.push_str(" AND d.account_id = ?");
    }
    if since.is_some() {
        sql.push_str(" AND d.created_at >= ?");
    }
    if until.is_some() {
        sql.push_str(" AND d.created_at <= ?");
    }
    let types = decision_types.filter(|t| !t.is_empty());
    if let Some(types) = types {
        let placeholders = vec!["?"; types.len()].join(", ");
        sql.push_str(&format!(" AND d.decision_type IN ({placeholders})"));
    }
    if impact.is_some() {
        sql.push_str(" AND d.impact = ?");
    }
    sql.push_str(" ORDER BY d.created_at DESC");
    if limit.is_some() {
        sql.push_str(" LIMIT ?");
    }
    if offset.is_some() {
        sql.push_str(" OFFSET ?");
    }

    let mut query = sqlx::query_as::<_, DecisionDbRow>(&sql);
    if let Some(account) = account_id {
        query = query.bind(account.to_string());
    }
    if let Some(since) = since {
        query = query.bind(since);
    }
    if let Some(until) = until {
        query = query.bind(until);
    }
    if let Some(types) = types {
        for t in types {
            query = query.bind(t);
        }
    }
    if let Some(impact) = impact {
        query = query.bind(impact.to_string());
    }
    if let Some(limit) = limit {
        query = query.bind(limit);
    }
    if let Some(offset) = offset {
        query = query.bind(offset);
    }

    let rows = query.fetch_all(db.pool()).await.map_err(map_sqlx_err)?;
    Ok(rows.into_iter().map(AiDecisionRow::from).collect())
}

/// Filtered, paginated decision list, newest first (F_E7 §4.4/§4.5).
pub async fn list_decisions(
    db: &Db,
    params: &ListDecisionsParams,
) -> AppResult<Vec<AiDecisionRow>> {
    let limit = params
        .limit
        .unwrap_or(LIST_DECISIONS_MAX_LIMIT)
        .clamp(1, LIST_DECISIONS_MAX_LIMIT);
    let offset = params.offset.unwrap_or(0).max(0);
    fetch_decisions(
        db,
        params.account_id.as_deref(),
        params.since_unix,
        params.until_unix,
        params.decision_types.as_deref(),
        params.impact.as_deref(),
        Some(limit),
        Some(offset),
    )
    .await
}

/// Window statistics in a single aggregate query (F_E7 §4.6) — no row loading.
pub async fn get_decisions_summary(
    db: &Db,
    account_id: Option<&str>,
    since: i64,
    until: i64,
) -> AppResult<DecisionSummary> {
    let row: (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), \
             COALESCE(SUM(CASE WHEN decision_type = 'auto_reply_sent' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(CASE WHEN decision_type = 'downgrade_e3_to_e2' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(CASE WHEN decision_type = 'sensitive_intercepted' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(CASE WHEN decision_type = 'draft_sent' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(CASE WHEN decision_type = 'draft_created' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(input_tokens), 0), \
             COALESCE(SUM(output_tokens), 0) \
         FROM ai_decisions \
         WHERE (? IS NULL OR account_id = ?) AND created_at >= ? AND created_at <= ?",
    )
    .bind(account_id)
    .bind(account_id)
    .bind(since)
    .bind(until)
    .fetch_one(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    let (
        total_events,
        auto_sent_count,
        downgrade_count,
        sensitive_count,
        draft_sent_count,
        draft_created_count,
        total_input_tokens,
        total_output_tokens,
    ) = row;
    let success_rate = if draft_created_count > 0 {
        draft_sent_count as f64 / draft_created_count as f64
    } else {
        0.0
    };
    Ok(DecisionSummary {
        total_events,
        auto_sent_count,
        downgrade_count,
        sensitive_count,
        draft_sent_count,
        draft_created_count,
        total_input_tokens,
        total_output_tokens,
        success_rate,
    })
}

/// Full (unpaginated) rows for the export window.
pub async fn export_decisions(
    db: &Db,
    params: &ExportAiDecisionsParams,
) -> AppResult<Vec<AiDecisionRow>> {
    fetch_decisions(
        db,
        params.account_id.as_deref(),
        Some(params.since_unix),
        Some(params.until_unix),
        None,
        None,
        None,
        None,
    )
    .await
}

// ── Export file writing ───────────────────────────────────────────────────────

/// The exported projection (F_E7 §4.7). `action_description`,
/// `result_description`, `knowledge_summary`, and `mail_subject` are excluded
/// by construction — they may carry mail-derived text (privacy boundary).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiDecisionExportRow<'a> {
    id: &'a str,
    account_id: &'a str,
    decision_type: &'a str,
    impact: &'a str,
    mail_id: Option<&'a str>,
    draft_id: Option<&'a str>,
    ai_model: Option<&'a str>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    latency_ms: Option<i64>,
    knowledge_refs: &'a [String],
    created_at: i64,
}

impl<'a> From<&'a AiDecisionRow> for AiDecisionExportRow<'a> {
    fn from(r: &'a AiDecisionRow) -> Self {
        AiDecisionExportRow {
            id: &r.id,
            account_id: &r.account_id,
            decision_type: &r.decision_type,
            impact: &r.impact,
            mail_id: r.mail_id.as_deref(),
            draft_id: r.draft_id.as_deref(),
            ai_model: r.ai_model.as_deref(),
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            latency_ms: r.latency_ms,
            knowledge_refs: &r.knowledge_refs,
            created_at: r.created_at,
        }
    }
}

/// Map export IO failures: out-of-space (ENOSPC) → `FS_DISK_FULL`, everything
/// else → `FS_PERMISSION_DENIED` (the exporter module's convention).
fn map_export_io(e: &std::io::Error, what: &str) -> AppError {
    if e.raw_os_error() == Some(28) {
        AppError::FsDiskFull
    } else {
        AppError::FsPermission(format!("{what}: {e}"))
    }
}

/// RFC-4180-style CSV escaping: quote when the field carries a comma, quote,
/// or newline; embedded quotes double.
fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

const CSV_HEADER: &str = "id,account_id,decision_type,impact,mail_id,draft_id,ai_model,\
input_tokens,output_tokens,latency_ms,knowledge_refs,created_at";

fn opt_num(v: Option<i64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}

/// Write the export rows to `ai_decisions_{unix_ts}.{csv|json}` under `dir`.
/// Pure with respect to state — unit-testable against a temp dir.
pub fn write_export_file(
    rows: &[AiDecisionRow],
    format: AiDecisionExportFormat,
    dir: &Path,
) -> AppResult<PathBuf> {
    let path = dir.join(format!(
        "ai_decisions_{}.{}",
        now_unix(),
        format.extension()
    ));
    let file = std::fs::File::create(&path).map_err(|e| map_export_io(&e, "create export file"))?;
    let mut writer = std::io::BufWriter::new(file);

    match format {
        AiDecisionExportFormat::Csv => {
            writeln!(writer, "{CSV_HEADER}").map_err(|e| map_export_io(&e, "write csv header"))?;
            for row in rows {
                let refs_json =
                    serde_json::to_string(&row.knowledge_refs).unwrap_or_else(|_| "[]".into());
                let line = [
                    csv_escape(&row.id),
                    csv_escape(&row.account_id),
                    csv_escape(&row.decision_type),
                    csv_escape(&row.impact),
                    csv_escape(row.mail_id.as_deref().unwrap_or_default()),
                    csv_escape(row.draft_id.as_deref().unwrap_or_default()),
                    csv_escape(row.ai_model.as_deref().unwrap_or_default()),
                    opt_num(row.input_tokens),
                    opt_num(row.output_tokens),
                    opt_num(row.latency_ms),
                    csv_escape(&refs_json),
                    row.created_at.to_string(),
                ]
                .join(",");
                writeln!(writer, "{line}").map_err(|e| map_export_io(&e, "write csv row"))?;
            }
        }
        AiDecisionExportFormat::Json => {
            let projected: Vec<AiDecisionExportRow<'_>> =
                rows.iter().map(AiDecisionExportRow::from).collect();
            let body = serde_json::to_string_pretty(&projected)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize export: {e}")))?;
            writer
                .write_all(body.as_bytes())
                .map_err(|e| map_export_io(&e, "write json export"))?;
        }
    }
    writer
        .flush()
        .map_err(|e| map_export_io(&e, "flush export"))?;
    Ok(path)
}

/// Query the window and write the export file under `{app data}/exports/`.
/// Returns the absolute file path. Errors: `FS_DISK_FULL`,
/// `FS_PERMISSION_DENIED`.
pub async fn export_decisions_to_file(
    state: &AppState,
    params: &ExportAiDecisionsParams,
) -> AppResult<String> {
    let rows = export_decisions(state.storage.db(), params).await?;
    let dir = state.paths.root.join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| map_export_io(&e, "create exports dir"))?;
    let path = write_export_file(&rows, params.format, &dir)?;
    // Identifiers and counts only (09 §5).
    tracing::info!(
        event = "ai_decisions_exported",
        row_count = rows.len(),
        format = params.format.extension(),
        "audit log export written"
    );
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::super::types::decision_type;
    use super::*;
    use crate::state::AppState;

    fn entry(account: &str, decision: &str) -> AuditEntry {
        AuditEntry {
            account_id: account.to_string(),
            mail_id: None,
            draft_id: None,
            decision_type: decision.to_string(),
            impact: "reply".into(),
            action_description: "Recorded an automated pipeline action.".into(),
            result_description: "Action completed.".into(),
            knowledge_refs: vec!["m-ref".into()],
            knowledge_summary: None,
            ai_model: Some("gpt-4o".into()),
            input_tokens: Some(100),
            output_tokens: Some(40),
            latency_ms: Some(900),
        }
    }

    async fn seed_account(state: &AppState) -> String {
        let id = crate::util::new_uuid();
        let now = now_unix();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, \
                 created_at, updated_at) VALUES (?, ?, 'Work', 'imap', 'slate', 'W', ?, ?)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
        id
    }

    async fn seed_mail(state: &AppState, id: &str, account_id: &str, subject: &str) {
        let now = now_unix();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_email, to_addrs, \
                 date_sent, date_received, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'sender@example.com', '[]', ?, ?, 0, 0)",
        )
        .bind(id)
        .bind(account_id)
        .bind(format!("<{id}@x>"))
        .bind(subject)
        .bind(now)
        .bind(now)
        .execute(state.storage.db().pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn log_await_inserts_append_only_rows() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;

        state
            .audit
            .log_await(entry(&account, decision_type::DRAFT_CREATED))
            .await
            .unwrap();
        state
            .audit
            .log_await(entry(&account, decision_type::DRAFT_CREATED))
            .await
            .unwrap();

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, decision_type FROM ai_decisions")
                .fetch_all(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(rows.len(), 2);
        // Append-only: two distinct row ids, never an upsert.
        assert_ne!(rows[0].0, rows[1].0);
        assert!(rows.iter().all(|(_, t)| t == "draft_created"));
    }

    #[tokio::test]
    async fn descriptions_are_truncated_to_200_chars() {
        let (state, _rx) = AppState::test_state().await;
        let account = seed_account(&state).await;
        let mut e = entry(&account, decision_type::DRAFT_CREATED);
        e.action_description = "a".repeat(500);
        e.result_description = "b".repeat(500);
        state.audit.log_await(e).await.unwrap();

        let (action, result): (String, String) =
            sqlx::query_as("SELECT action_description, result_description FROM ai_decisions")
                .fetch_one(state.storage.db().pool())
                .await
                .unwrap();
        assert_eq!(action.chars().count(), 200);
        assert_eq!(result.chars().count(), 200);
    }

    #[tokio::test]
    async fn list_filters_and_joins_mail_subject() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let account_a = seed_account(&state).await;
        let account_b = seed_account(&state).await;
        seed_mail(&state, "m1", &account_a, "Renewal terms").await;

        let mut with_mail = entry(&account_a, decision_type::DRAFT_CREATED);
        with_mail.mail_id = Some("m1".into());
        insert_decision(db, &with_mail).await.unwrap();
        insert_decision(db, &entry(&account_a, decision_type::DRAFT_SENT))
            .await
            .unwrap();
        insert_decision(db, &entry(&account_b, decision_type::DRAFT_SENT))
            .await
            .unwrap();

        // Account filter.
        let rows = list_decisions(
            db,
            &ListDecisionsParams {
                account_id: Some(account_a.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        let created = rows
            .iter()
            .find(|r| r.decision_type == "draft_created")
            .unwrap();
        assert_eq!(created.mail_subject.as_deref(), Some("Renewal terms"));
        assert_eq!(created.knowledge_refs, vec!["m-ref".to_string()]);

        // Decision-type filter.
        let rows = list_decisions(
            db,
            &ListDecisionsParams {
                decision_types: Some(vec!["draft_sent".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.decision_type == "draft_sent"));

        // Limit/offset.
        let rows = list_decisions(
            db,
            &ListDecisionsParams {
                limit: Some(1),
                offset: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn summary_aggregates_in_one_pass() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let account = seed_account(&state).await;
        for _ in 0..3 {
            insert_decision(db, &entry(&account, decision_type::DRAFT_CREATED))
                .await
                .unwrap();
        }
        for _ in 0..2 {
            insert_decision(db, &entry(&account, decision_type::DRAFT_SENT))
                .await
                .unwrap();
        }
        insert_decision(db, &entry(&account, decision_type::AUTO_REPLY_SENT))
            .await
            .unwrap();

        let now = now_unix();
        let summary = get_decisions_summary(db, None, now - 3_600, now + 3_600)
            .await
            .unwrap();
        assert_eq!(summary.total_events, 6);
        assert_eq!(summary.draft_created_count, 3);
        assert_eq!(summary.draft_sent_count, 2);
        assert_eq!(summary.auto_sent_count, 1);
        assert_eq!(summary.downgrade_count, 0);
        assert_eq!(summary.sensitive_count, 0);
        assert_eq!(summary.total_input_tokens, 600);
        assert_eq!(summary.total_output_tokens, 240);
        assert!((summary.success_rate - 2.0 / 3.0).abs() < 1e-9);

        // Empty window: all zero, no division by zero.
        let empty = get_decisions_summary(db, None, 0, 1).await.unwrap();
        assert_eq!(empty.total_events, 0);
        assert_eq!(empty.success_rate, 0.0);
    }

    #[tokio::test]
    async fn export_files_exclude_description_fields() {
        let (state, _rx) = AppState::test_state().await;
        let db = state.storage.db();
        let account = seed_account(&state).await;
        let mut e = entry(&account, decision_type::DRAFT_SENT);
        e.action_description = "SECRET-ACTION-TEXT".into();
        e.result_description = "SECRET-RESULT-TEXT".into();
        insert_decision(db, &e).await.unwrap();

        let now = now_unix();
        let params = ExportAiDecisionsParams {
            account_id: None,
            since_unix: now - 3_600,
            until_unix: now + 3_600,
            format: AiDecisionExportFormat::Csv,
        };
        let rows = export_decisions(db, &params).await.unwrap();
        assert_eq!(rows.len(), 1);

        let dir = std::env::temp_dir().join(format!("seekermail-audit-{}", new_uuid()));
        std::fs::create_dir_all(&dir).unwrap();

        // CSV: parseable, one data row, no description columns or values.
        let csv_path = write_export_file(&rows, AiDecisionExportFormat::Csv, &dir).unwrap();
        let csv = std::fs::read_to_string(&csv_path).unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], CSV_HEADER);
        assert!(!csv.contains("action_description"));
        assert!(!csv.contains("SECRET-ACTION-TEXT"));
        assert!(!csv.contains("SECRET-RESULT-TEXT"));
        assert_eq!(
            lines[1].split(',').count(),
            CSV_HEADER.split(',').count(),
            "data row must align with the header"
        );

        // JSON: parseable array, projected fields only.
        let json_path = write_export_file(&rows, AiDecisionExportFormat::Json, &dir).unwrap();
        let json = std::fs::read_to_string(&json_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert!(arr[0].get("actionDescription").is_none());
        assert!(arr[0].get("resultDescription").is_none());
        assert!(arr[0].get("knowledgeSummary").is_none());
        assert!(arr[0].get("mailSubject").is_none());
        assert_eq!(arr[0]["decisionType"], "draft_sent");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn csv_escaping_handles_quotes_commas_newlines() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape("two\nlines"), "\"two\nlines\"");
    }
}
