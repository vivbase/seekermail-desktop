-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 006 · F4 provider matrix (T065)
-- Forward-only, additive. `provider_matrix` holds the capability × account
-- routing JSON (CapabilityMatrix, F_F4 §4.4); NULL = not configured, the
-- registry falls back to the base `ai_provider` / `ai_model` columns.
-- =============================================================================

ALTER TABLE account_ai_settings ADD COLUMN provider_matrix TEXT;
