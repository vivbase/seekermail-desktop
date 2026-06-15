-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 011 · Attachment text extraction (T108)
-- Forward-only, additive. Adds the columns the extraction pipeline writes and a
-- denormalised `account_id` (copied from the owning mail) so the attachment
-- search index (T109) and cross-account search (T111/T112) need no JOIN.
--
-- NOTE on numbering: the cards drafted this as migration 008, but 008–010 were
-- already taken (im_messages / mail_processing_status / query_reminder). The
-- next free number is 011. SQLite has no `ADD COLUMN IF NOT EXISTS`; idempotency
-- is provided by sqlx's `_sqlx_migrations` ledger (each file runs exactly once).
-- =============================================================================

ALTER TABLE attachments ADD COLUMN extraction_status TEXT NOT NULL DEFAULT 'pending'
    CHECK (extraction_status IN ('pending', 'indexed', 'skipped', 'error'));

ALTER TABLE attachments ADD COLUMN extracted_text TEXT;

ALTER TABLE attachments ADD COLUMN extracted_at INTEGER;

-- Denormalised owning account, copied from the parent mail. Lets the attachment
-- FTS/vector index and cross-account search filter by account without a JOIN.
ALTER TABLE attachments ADD COLUMN account_id TEXT;

-- Backfill the denormalised account for every existing attachment row.
UPDATE attachments
SET account_id = (SELECT m.account_id FROM mails m WHERE m.id = attachments.mail_id)
WHERE account_id IS NULL;

-- Partial index over the work queue: only un-extracted, downloaded rows.
CREATE INDEX IF NOT EXISTS idx_attachments_extraction
    ON attachments(extraction_status)
    WHERE extraction_status = 'pending';

-- Account scoping for the attachment search index (T109/T111/T112).
CREATE INDEX IF NOT EXISTS idx_attachments_account
    ON attachments(account_id);
