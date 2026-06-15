-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 002 · Knowledge-depth (T016)
-- Forward-only, additive: adds two columns to `accounts`. Does NOT touch 001.
-- SQLite ADD COLUMN only appends, so this is safe + idempotent under sqlx.
-- See: function list/F_A1_multi_account.md §4.5
-- =============================================================================

-- NULL = "all mail"; 3 / 6 / 12 / 36 / 60 = last-N-months knowledge depth.
ALTER TABLE accounts ADD COLUMN knowledge_depth_months INTEGER;

-- Unix timestamp of the first depth selection (for "adjust range" UX later).
ALTER TABLE accounts ADD COLUMN knowledge_depth_set_at INTEGER;
