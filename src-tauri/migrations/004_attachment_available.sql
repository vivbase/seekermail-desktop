-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 004 · Attachment availability (T025)
-- Forward-only, additive. `available = 0` marks an attachment the server no
-- longer has (404 on FETCH), so auto-download stops retrying it (F_A5 §7).
-- =============================================================================

ALTER TABLE attachments ADD COLUMN available INTEGER NOT NULL DEFAULT 1;
