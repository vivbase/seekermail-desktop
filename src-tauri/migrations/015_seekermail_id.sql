-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 015
-- SeekerMail ID — local cache of the OPTIONAL, cloud-backed account identity (A6).
-- Applied by: sqlx::migrate!() at app startup (forward-only).
-- See: docs/function list/F_A6_seekermail_id.md (rewritten, binding-mailbox model
--      removed) and docs/analysis/26_identity_decoupling_and_email_marketing_foundation.md
-- =============================================================================
--
-- The SeekerMail ID is INDEPENDENT of imported mailboxes: there is deliberately NO
-- foreign key to `accounts`. It is created by signing in with Google (OIDC) and is
-- OPTIONAL — the app is fully usable locally with no row here (local-first). This
-- table holds AT MOST ONE row (single-row guard, id = 'self'): a local cache of the
-- identity the user signed in with, plus the OPT-IN marketing-consent flag. The
-- authoritative record (entitlement, devices, the marketing list) lives in the
-- SeekerMail cloud once that backend ships (T121).
--
-- INVARIANT: mail bodies, attachments, contacts, and GTE vectors NEVER appear here.
-- The `accounts` table is intentionally NOT modified by this migration (no
-- `is_id_binding` column) — that is the decoupling, expressed at the schema layer.
-- =============================================================================
CREATE TABLE IF NOT EXISTS seekermail_id (
    id                        TEXT    NOT NULL PRIMARY KEY DEFAULT 'self', -- single-row guard
    provider                  TEXT    NOT NULL DEFAULT 'google',  -- provider-agnostic; google at launch
    provider_subject          TEXT    NOT NULL,                   -- OIDC `sub` (stable user id)
    email                     TEXT    NOT NULL,                   -- identity email from the IdP
    email_verified            INTEGER NOT NULL DEFAULT 0,         -- boolean
    display_name              TEXT,
    plan                      TEXT,                               -- entitlement / subscription (transactional)
    marketing_consent         INTEGER NOT NULL DEFAULT 0,         -- boolean, OPT-IN (default OFF)
    marketing_consent_source  TEXT,                               -- 'onboarding_checkbox' | 'settings' | ...
    marketing_consent_at      INTEGER,                            -- unix ts consent given/withdrawn
    marketing_policy_version  TEXT,                               -- privacy-policy version shown at consent
    signed_in_at              INTEGER NOT NULL,                   -- unix ts of sign-in
    created_at                INTEGER NOT NULL,
    updated_at                INTEGER NOT NULL,
    CONSTRAINT seekermail_id_single_row CHECK (id = 'self')
);
