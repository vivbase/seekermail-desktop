-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 001
-- Applied by: sqlx::migrate!() at app startup
-- See: docs/dev/01_DATABASE_SCHEMA.md for full field documentation
-- =============================================================================

-- ─────────────────────────────────────────────────────────────────────────────
-- PRAGMA (applied per-connection in Rust, not here — listed for reference)
-- PRAGMA journal_mode = WAL;
-- PRAGMA foreign_keys = ON;
-- PRAGMA busy_timeout = 5000;
-- PRAGMA temp_store = MEMORY;
-- PRAGMA mmap_size = 134217728;
-- PRAGMA cache_size = -8000;
-- ─────────────────────────────────────────────────────────────────────────────


-- =============================================================================
-- ACCOUNTS
-- =============================================================================
CREATE TABLE IF NOT EXISTS accounts (
    id               TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    email            TEXT    NOT NULL,
    display_name     TEXT    NOT NULL,
    provider         TEXT    NOT NULL,              -- gmail | outlook | imap | exchange
    imap_host        TEXT,
    imap_port        INTEGER NOT NULL DEFAULT 993,
    smtp_host        TEXT,
    smtp_port        INTEGER NOT NULL DEFAULT 587,
    color_token      TEXT    NOT NULL,              -- terra | slate | sage
    badge_label      TEXT    NOT NULL,              -- L | W | P | custom single char
    role_type        TEXT    NOT NULL DEFAULT 'custom', -- legal | work | personal | sales | custom
    role_description TEXT,
    auth_level       INTEGER NOT NULL DEFAULT 1,    -- 1=E1 manual | 2=E2 semi | 3=E3 auto
    is_primary       INTEGER NOT NULL DEFAULT 0,    -- boolean, max one per db
    is_active        INTEGER NOT NULL DEFAULT 1,    -- boolean
    sync_interval_secs INTEGER NOT NULL DEFAULT 300,
    last_synced_at   INTEGER,
    imap_uid_validity INTEGER,
    imap_uid_next    INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_accounts_email
    ON accounts(email);


-- =============================================================================
-- THREADS
-- =============================================================================
CREATE TABLE IF NOT EXISTS threads (
    id              TEXT    NOT NULL PRIMARY KEY,   -- UUID v4
    account_id      TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    subject         TEXT    NOT NULL,               -- Re:/Fwd: stripped
    participants    TEXT    NOT NULL,               -- JSON array of email strings
    mail_count      INTEGER NOT NULL DEFAULT 1,
    unread_count    INTEGER NOT NULL DEFAULT 1,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    latest_date     INTEGER NOT NULL,               -- unix timestamp
    snippet         TEXT,                           -- 160-char preview
    is_archived     INTEGER NOT NULL DEFAULT 0,
    is_starred      INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_threads_account_date
    ON threads(account_id, latest_date DESC);

CREATE INDEX IF NOT EXISTS idx_threads_account_unread
    ON threads(account_id, unread_count)
    WHERE unread_count > 0;


-- =============================================================================
-- MAILS
-- =============================================================================
CREATE TABLE IF NOT EXISTS mails (
    id               TEXT    NOT NULL PRIMARY KEY,  -- UUID v4; also LanceDB row ID
    account_id       TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id        TEXT    REFERENCES threads(id) ON DELETE SET NULL,
    message_id       TEXT    NOT NULL,              -- RFC 2822 Message-ID header
    in_reply_to      TEXT,                          -- RFC 2822 In-Reply-To
    "references"     TEXT,                          -- space-sep References chain ("references" is a SQLite keyword — must be quoted)
    subject          TEXT    NOT NULL DEFAULT '',
    from_name        TEXT,
    from_email       TEXT    NOT NULL,              -- normalised lowercase
    to_addrs         TEXT    NOT NULL,              -- JSON: [{"name":"","email":""}]
    cc_addrs         TEXT    NOT NULL DEFAULT '[]',
    bcc_addrs        TEXT    NOT NULL DEFAULT '[]', -- sent items only
    reply_to         TEXT,                          -- JSON object or NULL
    date_sent        INTEGER NOT NULL,              -- unix timestamp
    date_received    INTEGER NOT NULL,
    body_text        TEXT,
    body_html        TEXT,                          -- B1-sanitised HTML
    snippet          TEXT,                          -- 200-char auto-preview
    is_read          INTEGER NOT NULL DEFAULT 0,
    is_starred       INTEGER NOT NULL DEFAULT 0,
    is_archived      INTEGER NOT NULL DEFAULT 0,
    is_deleted       INTEGER NOT NULL DEFAULT 0,    -- soft delete
    is_sent          INTEGER NOT NULL DEFAULT 0,
    is_draft_imap    INTEGER NOT NULL DEFAULT 0,    -- IMAP Draft folder
    folder           TEXT    NOT NULL DEFAULT 'INBOX',
    imap_uid         INTEGER,
    imap_flags       TEXT    NOT NULL DEFAULT '[]', -- JSON array of IMAP flag strings
    has_attachments  INTEGER NOT NULL DEFAULT 0,
    spam_score       REAL,                          -- 0.0–1.0; NULL = not scored
    tracker_blocked  INTEGER NOT NULL DEFAULT 0,
    tracker_count    INTEGER NOT NULL DEFAULT 0,
    embedding_status TEXT    NOT NULL DEFAULT 'pending', -- pending|indexed|skipped|error
    embedded_at      INTEGER,
    embedding_model  TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_mails_account_msgid
    ON mails(account_id, message_id);

CREATE INDEX IF NOT EXISTS idx_mails_account_date
    ON mails(account_id, date_sent DESC);

CREATE INDEX IF NOT EXISTS idx_mails_thread
    ON mails(thread_id);

CREATE INDEX IF NOT EXISTS idx_mails_from
    ON mails(from_email);

CREATE INDEX IF NOT EXISTS idx_mails_embedding_pending
    ON mails(embedding_status)
    WHERE embedding_status = 'pending';

CREATE INDEX IF NOT EXISTS idx_mails_unread
    ON mails(account_id, is_read)
    WHERE is_read = 0;


-- =============================================================================
-- ATTACHMENTS
-- =============================================================================
CREATE TABLE IF NOT EXISTS attachments (
    id               TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    mail_id          TEXT    NOT NULL REFERENCES mails(id) ON DELETE CASCADE,
    content_id       TEXT,                          -- RFC 2392 for inline images
    filename         TEXT    NOT NULL,
    content_type     TEXT    NOT NULL,
    size_bytes       INTEGER NOT NULL,
    checksum_sha256  TEXT,                          -- set after download
    local_path       TEXT,                          -- relative path in attachments dir
    downloaded       INTEGER NOT NULL DEFAULT 0,
    is_inline        INTEGER NOT NULL DEFAULT 0,
    downloaded_at    INTEGER,
    created_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_attachments_mail
    ON attachments(mail_id);

CREATE INDEX IF NOT EXISTS idx_attachments_pending
    ON attachments(downloaded)
    WHERE downloaded = 0;


-- =============================================================================
-- CONTACTS
-- =============================================================================
CREATE TABLE IF NOT EXISTS contacts (
    id                   TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    email                TEXT    NOT NULL,              -- normalised lowercase
    display_name         TEXT,
    organisation         TEXT,
    first_seen_at        INTEGER NOT NULL,
    last_seen_at         INTEGER NOT NULL,
    interaction_count    INTEGER NOT NULL DEFAULT 1,
    reply_count          INTEGER NOT NULL DEFAULT 0,
    avg_reply_hours      REAL,
    typical_hour_start   INTEGER,                       -- 0–23
    typical_hour_end     INTEGER,
    style_notes          TEXT,                          -- JSON AI-extracted style
    is_trusted           INTEGER NOT NULL DEFAULT 0,
    trust_score          REAL    NOT NULL DEFAULT 0.5,  -- 0.0–1.0
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_contacts_email
    ON contacts(email);

CREATE INDEX IF NOT EXISTS idx_contacts_trust
    ON contacts(trust_score DESC);


-- =============================================================================
-- AI DRAFTS  (E6 inline draft review in Pending)
-- =============================================================================
CREATE TABLE IF NOT EXISTS ai_drafts (
    id                   TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    trigger_mail_id      TEXT    NOT NULL REFERENCES mails(id) ON DELETE CASCADE,
    account_id           TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    to_addr              TEXT    NOT NULL,              -- JSON: {"name":"","email":""}
    cc_addrs             TEXT    NOT NULL DEFAULT '[]',
    subject              TEXT    NOT NULL,
    body_original        TEXT    NOT NULL,              -- AI output, immutable
    body_current         TEXT    NOT NULL,              -- user may edit
    is_edited            INTEGER NOT NULL DEFAULT 0,
    style_match_score    REAL,                          -- 0.0–1.0
    trigger_mode         TEXT    NOT NULL,              -- E1_manual|E2_semi|E3_auto
    ai_model             TEXT    NOT NULL,
    ai_prompt_version    TEXT,
    knowledge_refs       TEXT    NOT NULL DEFAULT '[]', -- JSON array of LanceDB mail_ids
    status               TEXT    NOT NULL DEFAULT 'pending', -- pending|edited|sent|discarded|expired
    send_after           INTEGER,                       -- E3: unix ts for delay queue
    expires_at           INTEGER,
    sent_at              INTEGER,
    discarded_at         INTEGER,
    discard_reason       TEXT,                          -- user|expired|superseded
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_drafts_account_pending
    ON ai_drafts(account_id, status)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_drafts_trigger_mail
    ON ai_drafts(trigger_mail_id);

CREATE INDEX IF NOT EXISTS idx_drafts_send_queue
    ON ai_drafts(send_after)
    WHERE status = 'pending' AND send_after IS NOT NULL;


-- =============================================================================
-- AI DECISIONS  (E7 Audit Log — append-only)
-- =============================================================================
CREATE TABLE IF NOT EXISTS ai_decisions (
    id                   TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    account_id           TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mail_id              TEXT    REFERENCES mails(id) ON DELETE SET NULL,
    draft_id             TEXT    REFERENCES ai_drafts(id) ON DELETE SET NULL,
    decision_type        TEXT    NOT NULL,
    -- auto_reply_sent | draft_created | risk_alert_t1..t6 |
    -- query_sent | auto_archived | contact_identified |
    -- style_updated | reindex_completed
    impact               TEXT    NOT NULL,              -- risk|reply|identity|rule|context
    action_description   TEXT    NOT NULL,
    knowledge_refs       TEXT    NOT NULL DEFAULT '[]',
    knowledge_summary    TEXT,                          -- human-readable, for UI
    result_description   TEXT    NOT NULL,
    ai_model             TEXT,
    input_tokens         INTEGER,
    output_tokens        INTEGER,
    latency_ms           INTEGER,
    created_at           INTEGER NOT NULL               -- immutable after insert
);

CREATE INDEX IF NOT EXISTS idx_decisions_account_date
    ON ai_decisions(account_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_decisions_mail
    ON ai_decisions(mail_id);

CREATE INDEX IF NOT EXISTS idx_decisions_today
    ON ai_decisions(created_at)
    WHERE created_at > (strftime('%s', 'now') - 86400);


-- =============================================================================
-- RISK EVENTS
-- =============================================================================
CREATE TABLE IF NOT EXISTS risk_events (
    id               TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    mail_id          TEXT    NOT NULL REFERENCES mails(id) ON DELETE CASCADE,
    account_id       TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    risk_level       INTEGER NOT NULL,              -- 1–6 (T1–T6)
    risk_type        TEXT    NOT NULL,
    -- domain_mismatch | payment_anomaly | identity_unknown |
    -- amount_threshold | rule_conflict | context_missing
    evidence         TEXT    NOT NULL,              -- JSON structured evidence
    description      TEXT    NOT NULL,
    status           TEXT    NOT NULL DEFAULT 'open', -- open|resolved|dismissed|expired
    resolution_note  TEXT,
    resolved_by      TEXT,                          -- user|timeout|superseded
    resolved_at      INTEGER,
    expires_at       INTEGER,                       -- NULL for T4 (never expires)
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_risk_account_open
    ON risk_events(account_id, status)
    WHERE status = 'open';

CREATE INDEX IF NOT EXISTS idx_risk_level_status
    ON risk_events(risk_level, status);


-- =============================================================================
-- PENDING QUERIES  (inquiry cards awaiting user response)
-- =============================================================================
CREATE TABLE IF NOT EXISTS pending_queries (
    id               TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    account_id       TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    mail_id          TEXT    REFERENCES mails(id) ON DELETE SET NULL,
    risk_event_id    TEXT    REFERENCES risk_events(id) ON DELETE SET NULL,
    trigger_type     TEXT    NOT NULL,              -- T1|T2|T3|T4|T5|T6
    question         TEXT    NOT NULL,
    options          TEXT,                          -- JSON array of option strings
    answer           TEXT,                          -- user's answer
    status           TEXT    NOT NULL DEFAULT 'pending', -- pending|answered|skipped|expired
    priority         INTEGER NOT NULL DEFAULT 3,    -- 1 (highest) – 5; T4 always = 1
    expires_at       INTEGER,                       -- NULL for T4
    answered_at      INTEGER,
    created_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_queries_pending
    ON pending_queries(account_id, priority, created_at)
    WHERE status = 'pending';


-- =============================================================================
-- SYNC STATE  (per-account IMAP sync bookmarks)
-- =============================================================================
CREATE TABLE IF NOT EXISTS sync_state (
    account_id            TEXT    NOT NULL PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    last_sync_at          INTEGER,
    last_sync_result      TEXT,                     -- ok|auth_error|network_error|partial
    consecutive_errors    INTEGER NOT NULL DEFAULT 0,
    backoff_until         INTEGER,
    inbox_uid_validity    INTEGER,
    inbox_uid_next        INTEGER,
    full_sync_required    INTEGER NOT NULL DEFAULT 1,
    total_mails_synced    INTEGER NOT NULL DEFAULT 0,
    updated_at            INTEGER NOT NULL
);


-- =============================================================================
-- SEARCH HISTORY
-- =============================================================================
CREATE TABLE IF NOT EXISTS search_history (
    id           INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id   TEXT    REFERENCES accounts(id) ON DELETE CASCADE, -- NULL = cross-account
    query        TEXT    NOT NULL,
    mode         TEXT    NOT NULL,                  -- keyword|semantic|structured
    result_count INTEGER,
    created_at   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_search_history_account
    ON search_history(account_id, created_at DESC);


-- =============================================================================
-- SAVED SEARCHES
-- =============================================================================
CREATE TABLE IF NOT EXISTS saved_searches (
    id           TEXT    NOT NULL PRIMARY KEY,      -- UUID v4
    account_id   TEXT    REFERENCES accounts(id) ON DELETE CASCADE, -- NULL = cross-account
    name         TEXT    NOT NULL,
    query        TEXT    NOT NULL,
    mode         TEXT    NOT NULL DEFAULT 'semantic',
    sort_order   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);


-- =============================================================================
-- APP SETTINGS  (global key-value store)
-- =============================================================================
CREATE TABLE IF NOT EXISTS app_settings (
    key        TEXT    NOT NULL PRIMARY KEY,
    value      TEXT    NOT NULL,                    -- JSON-encoded
    updated_at INTEGER NOT NULL
);

-- Seed defaults
INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES
    ('ui.theme',                    '"system"', strftime('%s', 'now')),  -- valid: light|dark|system (per 01_DATABASE_SCHEMA)
    ('ui.density',                  '"comfortable"', strftime('%s', 'now')),
    ('ui.language',                 '"en"',    strftime('%s', 'now')),
    ('notifications.global_level',  '"all"',   strftime('%s', 'now')),
    ('notifications.quiet_start',   '22',      strftime('%s', 'now')),
    ('notifications.quiet_end',     '8',       strftime('%s', 'now')),
    ('sync.background_enabled',     'true',    strftime('%s', 'now')),
    ('privacy.analytics_enabled',   'false',   strftime('%s', 'now')),
    ('gte.embedding_model',         '"bge-m3"', strftime('%s', 'now')),
    ('gte.index_spam',              'false',   strftime('%s', 'now')),
    ('onboarding.completed',        'false',   strftime('%s', 'now'));


-- =============================================================================
-- ACCOUNT AI SETTINGS
-- =============================================================================
CREATE TABLE IF NOT EXISTS account_ai_settings (
    account_id           TEXT    NOT NULL PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    auth_level           INTEGER NOT NULL DEFAULT 1,    -- mirrors accounts.auth_level
    ai_provider          TEXT    NOT NULL DEFAULT 'none', -- openai|anthropic|ollama|local_onnx|none
    ai_model             TEXT,
    ai_api_key_ref       TEXT,                          -- Keychain item name (NOT the key)
    ai_base_url          TEXT,                          -- for self-hosted / Ollama
    t1_enabled           INTEGER NOT NULL DEFAULT 1,
    t2_enabled           INTEGER NOT NULL DEFAULT 1,
    t3_enabled           INTEGER NOT NULL DEFAULT 1,
    t4_enabled           INTEGER NOT NULL DEFAULT 1,    -- T4 cannot have permanent expiry removed
    t5_enabled           INTEGER NOT NULL DEFAULT 0,
    t6_enabled           INTEGER NOT NULL DEFAULT 1,
    daily_query_limit    INTEGER NOT NULL DEFAULT 10,
    e3_whitelist_only    INTEGER NOT NULL DEFAULT 1,    -- E3 auto-send to known contacts only
    e3_min_history       INTEGER NOT NULL DEFAULT 3,    -- min prior interactions
    style_profile        TEXT,                          -- JSON learned style features
    style_samples_count  INTEGER NOT NULL DEFAULT 0,
    updated_at           INTEGER NOT NULL
);


-- =============================================================================
-- FTS5 VIRTUAL TABLE  (C1 — Keyword Search)
-- =============================================================================
CREATE VIRTUAL TABLE IF NOT EXISTS mails_fts USING fts5(
    subject,
    body_text,
    from_name,
    from_email,
    content     = 'mails',
    content_rowid = 'rowid',
    tokenize    = 'unicode61 remove_diacritics 1'
);

-- Keep FTS in sync with mails table
CREATE TRIGGER IF NOT EXISTS mails_fts_after_insert
AFTER INSERT ON mails BEGIN
    INSERT INTO mails_fts(rowid, subject, body_text, from_name, from_email)
    VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_email);
END;

CREATE TRIGGER IF NOT EXISTS mails_fts_after_delete
AFTER DELETE ON mails BEGIN
    INSERT INTO mails_fts(mails_fts, rowid, subject, body_text, from_name, from_email)
    VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_email);
END;

CREATE TRIGGER IF NOT EXISTS mails_fts_after_update
AFTER UPDATE ON mails BEGIN
    INSERT INTO mails_fts(mails_fts, rowid, subject, body_text, from_name, from_email)
    VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_email);
    INSERT INTO mails_fts(rowid, subject, body_text, from_name, from_email)
    VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_email);
END;

-- =============================================================================
-- END OF MIGRATION 001
-- =============================================================================
