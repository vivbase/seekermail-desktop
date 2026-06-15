-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 012 · Attachment FTS5 + embedding (T109)
-- Forward-only, additive. Adds:
--   * `attachments_fts` — an external-content FTS5 table mirroring mails_fts, but
--     keyed on the attachment's filename + extracted text;
--   * triggers that fill it when a row turns `extraction_status = 'indexed'` and
--     clear it on delete;
--   * `embedding_att_status` — the per-attachment vector-index state.
--
-- Numbering: drafted as 009 in the card, but 009 was taken; next free is 012.
-- The FTS insert trigger fires on UPDATE (not INSERT) because `extracted_text`
-- is NULL at insert time and only set when extraction completes (T109 §6).
-- =============================================================================

ALTER TABLE attachments ADD COLUMN embedding_att_status TEXT NOT NULL DEFAULT 'pending'
    CHECK (embedding_att_status IN ('pending', 'indexed', 'skipped', 'error'));

CREATE VIRTUAL TABLE IF NOT EXISTS attachments_fts USING fts5(
    filename,
    extracted_text,
    content       = 'attachments',
    content_rowid = 'rowid',
    tokenize      = 'unicode61 remove_diacritics 1'
);

-- Fill FTS only when a row becomes `indexed` (extracted_text is populated then).
CREATE TRIGGER IF NOT EXISTS attachments_fts_after_index
AFTER UPDATE OF extraction_status ON attachments
WHEN new.extraction_status = 'indexed'
BEGIN
    INSERT INTO attachments_fts(rowid, filename, extracted_text)
    VALUES (new.rowid, new.filename, new.extracted_text);
END;

-- Keep FTS aligned on delete (account/mail cascade removes attachment rows).
CREATE TRIGGER IF NOT EXISTS attachments_fts_after_delete
AFTER DELETE ON attachments BEGIN
    INSERT INTO attachments_fts(attachments_fts, rowid, filename, extracted_text)
    VALUES ('delete', old.rowid, old.filename, old.extracted_text);
END;

-- Work queue for the vector-embedding phase (T109 §3b).
CREATE INDEX IF NOT EXISTS idx_attachments_embedding
    ON attachments(embedding_att_status)
    WHERE embedding_att_status = 'pending';
