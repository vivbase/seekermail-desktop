-- Migration 001: identity + sessions tables
-- SeekerMail cloud identity service (T121a)
--
-- Rules:
--   * identity stores WHO the user is and WHAT plan they are on.
--   * sessions are opaque bearer tokens issued after OIDC verification.
--   * NOTHING here touches mail bodies, attachments, contacts, or GTE vectors.

CREATE TABLE IF NOT EXISTS identities (
    id                      UUID        PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Google OIDC fields (scope: openid email profile ONLY)
    provider                TEXT        NOT NULL DEFAULT 'google',
    provider_subject        TEXT        NOT NULL,          -- Google "sub" claim
    email                   TEXT        NOT NULL,
    email_verified          BOOLEAN     NOT NULL DEFAULT FALSE,
    display_name            TEXT,

    -- Subscription entitlement
    plan                    TEXT        NOT NULL DEFAULT 'free',  -- 'free' | 'pro'

    -- Marketing consent (opt-IN, default OFF, first-party only)
    marketing_consent       BOOLEAN     NOT NULL DEFAULT FALSE,
    marketing_consent_at    TIMESTAMPTZ,

    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT identities_provider_subject_unique UNIQUE (provider, provider_subject)
);

CREATE TABLE IF NOT EXISTS sessions (
    token                   TEXT        PRIMARY KEY,       -- opaque hex-encoded 32-byte random
    identity_id             UUID        NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
    device_name             TEXT        NOT NULL DEFAULT 'desktop',
    expires_at              TIMESTAMPTZ NOT NULL,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS sessions_identity_id_idx ON sessions (identity_id);
CREATE INDEX IF NOT EXISTS sessions_expires_at_idx  ON sessions (expires_at);

-- Auto-update updated_at on identities
CREATE OR REPLACE FUNCTION touch_updated_at()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

CREATE TRIGGER identities_updated_at
    BEFORE UPDATE ON identities
    FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
