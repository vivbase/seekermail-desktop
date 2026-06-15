-- =============================================================================
-- 009_mail_processing_status.sql — I3 proactive-query processing state (T095)
--
-- Forward-only (08 §migrations). The card called this "005"; migrations 005–008
-- already exist, so it lands as 009. Adds a column tracking where a mail sits in
-- the AI processing lifecycle so I3 can suspend the E1/E2/E3 chain pending a
-- human answer, and resume it later (T096).
--   none        — not yet processed by the AI pipeline
--   analyzing   — re-queued after a query was answered (T096)
--   suspended_i3— a proactive query was raised; the E-chain is paused
--   done        — terminal (skipped / expired / completed)
-- =============================================================================

ALTER TABLE mails ADD COLUMN ai_processing_status TEXT NOT NULL DEFAULT 'none';

CREATE INDEX IF NOT EXISTS idx_mails_suspended
    ON mails(ai_processing_status)
    WHERE ai_processing_status = 'suspended_i3';
