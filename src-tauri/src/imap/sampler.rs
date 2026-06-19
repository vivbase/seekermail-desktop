//! Mailbox sampling for knowledge-depth selection (T016).
//!
//! After a successful connection, count messages per time bucket via
//! `SEARCH SINCE` (UID counts only — never FETCH bodies, F_A1 §4.5.1) to size each
//! depth option. Bounded by a 10 s total timeout; on timeout the counts degrade to
//! `None` and the command still returns `Ok` (in-band).

use std::time::Duration;

use crate::config::SAMPLING_TIMEOUT_SECS;
use crate::error::AppResult;
use crate::state::AppState;
use crate::types::{RangeEstimate, SamplingResult};
use crate::util::now_unix;

/// The six depth buckets shown in the wizard (months; `None` = all mail).
const BUCKETS: [Option<u32>; 6] = [Some(3), Some(6), Some(12), Some(36), Some(60), None];

const MONTH_SECS: i64 = 30 * 86_400;

/// Average on-disk bytes per message including attachments. Chosen (≈300 KB) to
/// match the spec's acceptance range (9,600 mails ⇒ ~2–4 GB) and the UI example
/// — reconciling the card's internally inconsistent formula vs. test (T016 §6/§8).
const AVG_BYTES_PER_MAIL: u64 = 300 * 1024;

/// Sample the mailbox. Always `Ok`; counts are `None` on timeout/failure.
pub async fn sample_mailbox(state: &AppState, account_id: &str) -> AppResult<SamplingResult> {
    let work = collect(state, account_id);
    match tokio::time::timeout(Duration::from_secs(SAMPLING_TIMEOUT_SECS), work).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(_)) | Err(_) => Ok(degraded()),
    }
}

async fn collect(state: &AppState, account_id: &str) -> AppResult<SamplingResult> {
    let creds = super::sync::imap_creds_for(state, account_id).await?;
    let mut session = state.net.imap.open(creds).await?;
    let _ = session.select_inbox().await?;

    let now = now_unix();
    let mut ranges = Vec::with_capacity(BUCKETS.len());
    for months in BUCKETS {
        let boundary = months.map(|m| now - (m as i64) * MONTH_SECS).unwrap_or(0);
        let count = session.search_uids_since(boundary).await?.len() as u32;
        let estimated_mb = ((count as u64 * AVG_BYTES_PER_MAIL) / (1024 * 1024)) as u32;
        ranges.push(RangeEstimate {
            months,
            mail_count: Some(count),
            estimated_mb: Some(estimated_mb),
        });
    }
    Ok(SamplingResult { ranges })
}

/// All buckets present, all counts unknown (timeout/failure path).
fn degraded() -> SamplingResult {
    SamplingResult {
        ranges: BUCKETS
            .iter()
            .map(|&months| RangeEstimate {
                months,
                mail_count: None,
                estimated_mb: None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn offline_sample_degrades_in_band() {
        let (state, _rx) = AppState::test_state().await;
        // Offline imap factory → connect fails → degraded result, still Ok.
        let res = sample_mailbox(&state, "missing-account").await.unwrap();
        assert_eq!(res.ranges.len(), 6);
        assert!(res.ranges.iter().all(|r| r.mail_count.is_none()));
    }

    #[tokio::test]
    async fn sample_counts_buckets_against_a_fake_mailbox() {
        // The success path — connect → SELECT → SEARCH SINCE per bucket — is
        // unreachable through the offline adapter; drive it with a fake mailbox.
        use crate::net::fakes::{net_with_imap, FakeImapFactory, FakeMailbox};

        let account_id = "5f2d6a1e-0000-4000-8000-000000000001";
        let mailbox = FakeMailbox::new()
            .with_inbox(1, 200, 3)
            .with_uids([101, 102, 103]);
        let (state, _rx) =
            AppState::test_state_with_net(net_with_imap(FakeImapFactory::new(mailbox))).await;

        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, provider, imap_host, color_token, \
                 badge_label, created_at, updated_at) \
             VALUES (?, 'a@x.com', 'A', 'imap', 'imap.example.com', 'slate', 'A', 0, 0)",
        )
        .bind(account_id)
        .execute(state.storage.db().pool())
        .await
        .unwrap();

        let res = sample_mailbox(&state, account_id).await.unwrap();
        assert_eq!(res.ranges.len(), 6);
        // The fake returns all three UIDs for every SINCE bucket → every bucket
        // counts 3, and the size estimate is populated (not the degraded `None`).
        assert!(res.ranges.iter().all(|r| r.mail_count == Some(3)));
        assert!(res.ranges.iter().all(|r| r.estimated_mb.is_some()));
    }

    #[test]
    fn estimate_in_expected_range() {
        // 9,600 mails should land in the 2,000–4,000 MB band (T016 §8).
        let mb = ((9600u64 * AVG_BYTES_PER_MAIL) / (1024 * 1024)) as u32;
        assert!((2000..=4000).contains(&mb), "got {mb} MB");
    }
}
