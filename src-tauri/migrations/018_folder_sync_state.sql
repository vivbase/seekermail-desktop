-- 018_folder_sync_state.sql
-- Per-folder IMAP sync cursors for multi-folder fetch (SENT / JUNK / TRASH
-- alongside INBOX). IMAP UIDVALIDITY and UIDNEXT are per-folder, so each
-- (account, folder) needs its own cursor; account-level health (errors, backoff,
-- auth) stays in `sync_state`. Forward-only and additive: the existing
-- single-folder INBOX cursor is seeded here so the proven INBOX sync keeps its
-- high-water mark on upgrade. Legacy `sync_state.inbox_uid_*` columns are left in
-- place (a later card moves the INBOX path onto this table).

CREATE TABLE IF NOT EXISTS folder_sync_state (
    account_id          TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder              TEXT    NOT NULL,   -- INBOX | SENT | JUNK | TRASH | DRAFTS | ARCHIVE
    uid_validity        INTEGER,
    uid_next            INTEGER,
    full_sync_required  INTEGER NOT NULL DEFAULT 1,
    total_mails_synced  INTEGER NOT NULL DEFAULT 0,
    last_sync_at        INTEGER,
    updated_at          INTEGER NOT NULL,
    PRIMARY KEY (account_id, folder)
);

-- Seed each existing account's INBOX row from the legacy single-folder cursor so
-- an in-place upgrade does not lose the incremental high-water mark. New accounts
-- created after this migration get their INBOX row from the sync scheduler's
-- per-folder `ensure`, so this one-shot copy only matters for existing installs.
INSERT OR IGNORE INTO folder_sync_state
    (account_id, folder, uid_validity, uid_next, full_sync_required,
     total_mails_synced, last_sync_at, updated_at)
SELECT account_id, 'INBOX', inbox_uid_validity, inbox_uid_next, full_sync_required,
       total_mails_synced, last_sync_at, CAST(strftime('%s', 'now') AS INTEGER)
FROM sync_state;
