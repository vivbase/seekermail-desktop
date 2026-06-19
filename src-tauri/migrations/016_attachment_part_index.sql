-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 016 · Attachment MIME part index (T025)
-- Forward-only, additive. Stores the attachment's ordinal position within the
-- parsed MIME message (0-based, as yielded by the parser's `attachments()`
-- iterator) so the deferred byte-download (`fetch_part`) can re-address the exact
-- part on the server instead of guessing.
--
-- Background: the v0.2 download path passed the attachment row's UUID as the IMAP
-- part specifier, which is never a valid part id — so a real `FETCH BODY.PEEK[..]`
-- could not locate the bytes. Persisting the stable part index resolves this.
--
-- Legacy rows (created before this migration) default to 0; they are re-resolved
-- on the next sync of their mail. There is no production data yet (pre-v0.1), so
-- the default is harmless.
-- =============================================================================

ALTER TABLE attachments ADD COLUMN part_index INTEGER NOT NULL DEFAULT 0;
