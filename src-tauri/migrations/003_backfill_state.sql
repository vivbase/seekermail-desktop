-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 003 · History backfill state (T022)
-- Forward-only: new table only. Does NOT modify 001 / 002.
-- See: function list/F_A4_mail_polling.md §3–§5
-- =============================================================================

CREATE TABLE IF NOT EXISTS backfill_state (
    account_id        TEXT    NOT NULL PRIMARY KEY
                              REFERENCES accounts(id) ON DELETE CASCADE,
    status            TEXT    NOT NULL DEFAULT 'idle',  -- idle|running|paused|completed|error
    depth_months      INTEGER,                          -- snapshot of knowledge depth at start; NULL=all
    boundary_date     INTEGER,                          -- unix ts: now - depth_months*30d (0/NULL = all)
    last_uid_fetched  INTEGER,                          -- resume cursor: lowest UID already persisted
    total_uid_count   INTEGER,                          -- SEARCH result size, for progress %
    fetched_count     INTEGER NOT NULL DEFAULT 0,       -- UIDs successfully persisted so far
    started_at        INTEGER,
    paused_at         INTEGER,
    completed_at      INTEGER,
    error_message     TEXT,
    updated_at        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_backfill_status
    ON backfill_state(status);
