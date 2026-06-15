-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 005
-- Compose drafts (T045). `ai_drafts` is reserved for AI-generated replies (E6,
-- with a NOT NULL trigger_mail_id FK), so user-authored compose drafts get their
-- own standalone table. Forward-only; never edit 001_init.sql.
-- =============================================================================

CREATE TABLE IF NOT EXISTS compose_drafts (
    id            TEXT    NOT NULL PRIMARY KEY,          -- UUID v4
    account_id    TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    to_addrs      TEXT    NOT NULL DEFAULT '[]',          -- JSON [{name,email}]
    cc_addrs      TEXT    NOT NULL DEFAULT '[]',
    subject       TEXT    NOT NULL DEFAULT '',
    body_text     TEXT    NOT NULL DEFAULT '',
    body_html     TEXT,
    in_reply_to   TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_compose_drafts_account
    ON compose_drafts(account_id, updated_at DESC);
