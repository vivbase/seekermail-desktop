-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 007 · fix ai_decisions partial index
--
-- 001 created `idx_decisions_today` as a partial index whose WHERE clause
-- calls strftime('%s','now'). SQLite (3.40+) rejects every INSERT into a table
-- carrying a non-deterministic partial index with:
--     "non-deterministic use of strftime() in an index"
-- which made `ai_decisions` effectively append-proof. Replace it with a plain
-- created_at index; the registry's daily-limit counter and the T069 24h
-- summary both filter on created_at and are served fine by a normal index.
-- =============================================================================

DROP INDEX IF EXISTS idx_decisions_today;

CREATE INDEX IF NOT EXISTS idx_decisions_created
    ON ai_decisions(created_at);
