-- 019_outbound_ops.sql
-- Outbound write-back queue (Phase 2 two-way sync). A local action records a
-- durable op here; the drain worker (`imap::outbound`) applies it to the IMAP
-- server and marks it done. Flag ops use `UID STORE`; relocation ops use
-- `UID MOVE`. Durable so an action survives a restart or an offline window;
-- re-applying a claimed-but-uncommitted op is harmless (`STORE +/-FLAGS` is
-- idempotent; a re-tried MOVE of an already-moved UID just fails and retries).

CREATE TABLE IF NOT EXISTS outbound_ops (
    id           TEXT    NOT NULL PRIMARY KEY,
    account_id   TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder       TEXT    NOT NULL,           -- local folder tag (INBOX|SENT|JUNK|TRASH); the drain maps it to the live server mailbox name
    imap_uid     INTEGER NOT NULL,
    op_type      TEXT    NOT NULL,           -- flags: mark_seen|mark_unseen|flag|unflag · moves: archive|trash|mark_spam|restore
    status       TEXT    NOT NULL DEFAULT 'pending',  -- pending | done | failed
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

-- The drain worker scans pending ops oldest-first per account.
CREATE INDEX IF NOT EXISTS idx_outbound_ops_pending
    ON outbound_ops(account_id, created_at)
    WHERE status = 'pending';
