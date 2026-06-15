//! Sent-mail sampling for style learning (T075 §3, F_E5 §4.1).
//!
//! Pulls the account's sent mails from the last 180 days and keeps only
//! qualifying ones: body length within [30, 2000] chars, not a forward, not an
//! auto-reply chain, fewer than 10 recipients. When more than 200 qualify, the
//! list is thinned by uniform date-ordered sampling so the kept set still
//! spans the whole window.
//!
//! Log safety (09 §5): only `account_id` and counts are logged — never
//! subjects, bodies, or addresses.

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::storage::{map_sqlx_err, Db};
use crate::util::{now_unix, truncate_chars};

use super::{
    BODY_MAX_CHARS, BODY_MIN_CHARS, BODY_TRIM_CHARS, MAX_RECIPIENTS, MAX_SAMPLES,
    SAMPLE_WINDOW_SECS,
};

/// One qualifying sent mail, body pre-trimmed to [`BODY_TRIM_CHARS`] chars for
/// token control (T075 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleSample {
    pub mail_id: String,
    pub date_sent: i64,
    pub subject: String,
    pub body_text_trimmed: String,
}

/// Select the account's style-learning corpus per F_E5 §4.1. Returns at most
/// [`MAX_SAMPLES`] samples ordered by `date_sent` ascending.
pub async fn sample_sent_mails(db: &Db, account_id: &str) -> AppResult<Vec<StyleSample>> {
    let window_start = now_unix() - SAMPLE_WINDOW_SECS;
    let rows: Vec<(String, i64, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, date_sent, subject, body_text, to_addrs FROM mails \
         WHERE account_id = ? AND is_sent = 1 AND is_deleted = 0 AND date_sent > ? \
         ORDER BY date_sent ASC, id ASC",
    )
    .bind(account_id)
    .bind(window_start)
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx_err)?;

    let qualifying: Vec<StyleSample> = rows
        .into_iter()
        .filter_map(|(id, date_sent, subject, body, to_addrs)| {
            let body = body?;
            if !qualifies(&subject, &body, &to_addrs) {
                return None;
            }
            Some(StyleSample {
                mail_id: id,
                date_sent,
                subject,
                body_text_trimmed: truncate_chars(&body, BODY_TRIM_CHARS),
            })
        })
        .collect();

    let total = qualifying.len();
    let picked: Vec<StyleSample> = if total > MAX_SAMPLES {
        let keep = uniform_indices(total, MAX_SAMPLES);
        let mut iter = qualifying.into_iter();
        let mut out = Vec::with_capacity(MAX_SAMPLES);
        let mut cursor = 0usize;
        for idx in keep {
            // `keep` is strictly increasing, so a single forward pass suffices.
            let sample = iter.nth(idx - cursor).expect("index within corpus");
            cursor = idx + 1;
            out.push(sample);
        }
        out
    } else {
        qualifying
    };

    tracing::info!(
        event = "style_samples_selected",
        account_id = account_id,
        qualifying = total,
        sample_count = picked.len(),
        "sent-mail style corpus selected"
    );
    Ok(picked)
}

/// The pure per-mail filter (F_E5 §4.1):
/// body length ∈ [30, 2000] chars; subject not a forward (`Fwd:`) and not a
/// deep reply chain (`Re: Re: Re:` — the auto-reply heuristic); fewer than 10
/// `to` recipients.
pub(crate) fn qualifies(subject: &str, body: &str, to_addrs_json: &str) -> bool {
    let len = body.chars().count();
    if !(BODY_MIN_CHARS..=BODY_MAX_CHARS).contains(&len) {
        return false;
    }
    let s = subject.trim().to_ascii_lowercase();
    if s.starts_with("fwd:") || s.starts_with("re: re: re:") {
        return false;
    }
    let recipients = serde_json::from_str::<serde_json::Value>(to_addrs_json)
        .ok()
        .and_then(|v| v.as_array().map(|a| a.len()))
        .unwrap_or(0);
    recipients < MAX_RECIPIENTS
}

/// `max` strictly-increasing indices spread uniformly over `0..n` (identity
/// when `n <= max`). Shared by the sampler cap and the snippet picker.
pub fn uniform_indices(n: usize, max: usize) -> Vec<usize> {
    if n <= max {
        return (0..n).collect();
    }
    let step = n as f64 / max as f64; // > 1.0, so floors strictly increase
    (0..max)
        .map(|k| ((k as f64 * step).floor() as usize).min(n - 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::new_uuid;

    async fn db() -> Db {
        let db = Db::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_account(db: &Db) -> String {
        let id = new_uuid();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, color_token, badge_label, created_at, updated_at) \
             VALUES (?, ?, 'Work', 'imap', 'slate', 'W', 0, 0)",
        )
        .bind(&id)
        .bind(format!("{id}@example.com"))
        .execute(db.pool())
        .await
        .unwrap();
        id
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_mail(
        db: &Db,
        account_id: &str,
        tag: &str,
        subject: &str,
        body: &str,
        recipients: usize,
        is_sent: bool,
        date_sent: i64,
    ) {
        let to: Vec<serde_json::Value> = (0..recipients)
            .map(|k| {
                let email = format!("recipient{k}@example.com");
                serde_json::json!({ "name": null, "email": email })
            })
            .collect();
        sqlx::query(
            "INSERT INTO mails (id, account_id, message_id, subject, from_email, to_addrs, \
             date_sent, date_received, body_text, is_sent, folder, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 'me@example.com', ?, ?, ?, ?, ?, ?, 0, 0)",
        )
        .bind(format!("{account_id}-{tag}"))
        .bind(account_id)
        .bind(format!("<{tag}@{account_id}>"))
        .bind(subject)
        .bind(serde_json::to_string(&to).unwrap())
        .bind(date_sent)
        .bind(date_sent)
        .bind(body)
        .bind(is_sent as i64)
        .bind(if is_sent { "Sent" } else { "INBOX" })
        .execute(db.pool())
        .await
        .unwrap();
    }

    const GOOD_BODY: &str = "Hi Priya,\n\nQuick update on the quarterly filing. The auditors \
        confirmed the inventory adjustment, so we are clear to close the books on Friday. \
        I attached the reconciliation summary for your records.\n\nBest regards,\nMaya";

    #[tokio::test]
    async fn qualifying_sent_mails_are_returned_in_date_order() {
        let db = db().await;
        let account = seed_account(&db).await;
        let base = now_unix() - 10 * 86_400;
        for i in 0..5 {
            insert_mail(
                &db,
                &account,
                &format!("ok-{i}"),
                &format!("Re: Quarterly filing {i}"),
                GOOD_BODY,
                1,
                true,
                base + i * 3_600,
            )
            .await;
        }
        let samples = sample_sent_mails(&db, &account).await.unwrap();
        assert_eq!(samples.len(), 5);
        assert!(samples.windows(2).all(|w| w[0].date_sent <= w[1].date_sent));
        assert!(samples
            .iter()
            .all(|s| s.body_text_trimmed.chars().count() <= BODY_TRIM_CHARS));
    }

    #[tokio::test]
    async fn nonqualifying_mails_are_filtered_out() {
        let db = db().await;
        let account = seed_account(&db).await;
        let recent = now_unix() - 5 * 86_400;
        // Repeating pushes the body well past the 2000-char ceiling.
        let long_body = "Every clause of the renewal needs a second read. ".repeat(60);
        let chain = "Re: Re: Re: Renewal terms";

        insert_mail(
            &db,
            &account,
            "keep",
            "Re: Renewal terms",
            GOOD_BODY,
            2,
            true,
            recent,
        )
        .await;
        insert_mail(
            &db,
            &account,
            "fwd",
            "Fwd: Renewal terms",
            GOOD_BODY,
            1,
            true,
            recent,
        )
        .await;
        insert_mail(&db, &account, "chain", chain, GOOD_BODY, 1, true, recent).await;
        insert_mail(
            &db,
            &account,
            "in",
            "Re: Renewal terms",
            GOOD_BODY,
            1,
            false,
            recent,
        )
        .await;
        insert_mail(
            &db,
            &account,
            "short",
            "Re: Renewal",
            "Sounds good!",
            1,
            true,
            recent,
        )
        .await;
        insert_mail(
            &db,
            &account,
            "long",
            "Re: Renewal terms",
            &long_body,
            1,
            true,
            recent,
        )
        .await;
        insert_mail(
            &db,
            &account,
            "blast",
            "Re: Renewal",
            GOOD_BODY,
            10,
            true,
            recent,
        )
        .await;
        insert_mail(
            &db,
            &account,
            "old",
            "Re: Renewal terms",
            GOOD_BODY,
            1,
            true,
            now_unix() - 200 * 86_400, // outside the 180-day window
        )
        .await;

        let samples = sample_sent_mails(&db, &account).await.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].mail_id, format!("{account}-keep"));
    }

    #[tokio::test]
    async fn corpus_over_cap_is_uniformly_thinned_to_max() {
        let db = db().await;
        let account = seed_account(&db).await;
        let base = now_unix() - 170 * 86_400;
        for i in 0..250i64 {
            insert_mail(
                &db,
                &account,
                &format!("bulk-{i:03}"),
                &format!("Re: Weekly status {i}"),
                GOOD_BODY,
                1,
                true,
                base + i * 3_600,
            )
            .await;
        }
        let samples = sample_sent_mails(&db, &account).await.unwrap();
        assert_eq!(samples.len(), MAX_SAMPLES);
        // Strictly increasing dates → uniform pass kept order and uniqueness.
        assert!(samples.windows(2).all(|w| w[0].date_sent < w[1].date_sent));
        // The kept set spans the window: the first mail survives thinning.
        assert_eq!(samples[0].mail_id, format!("{account}-bulk-000"));
    }

    #[test]
    fn qualifies_filter_matrix() {
        let one = r#"[{"name":null,"email":"a@x.y"}]"#;
        assert!(qualifies("Re: Renewal", GOOD_BODY, one));
        assert!(!qualifies("Fwd: Renewal", GOOD_BODY, one));
        assert!(!qualifies("  fwd: renewal", GOOD_BODY, one));
        assert!(!qualifies("Re: Re: Re: Renewal", GOOD_BODY, one));
        assert!(!qualifies("Re: Renewal", "Too short.", one));
        // Malformed recipient JSON counts as zero recipients, not a group send.
        assert!(qualifies("Re: Renewal", GOOD_BODY, "not json"));
    }

    #[test]
    fn uniform_indices_is_identity_under_cap_and_uniform_over_it() {
        assert_eq!(uniform_indices(5, 200), vec![0, 1, 2, 3, 4]);
        let idx = uniform_indices(250, 200);
        assert_eq!(idx.len(), 200);
        assert_eq!(idx[0], 0);
        assert!(idx.windows(2).all(|w| w[0] < w[1]), "strictly increasing");
        assert!(*idx.last().unwrap() < 250);
    }
}
