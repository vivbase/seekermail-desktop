-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 013 · Deal tags (G5 transaction view, T119)
-- Forward-only, additive. Human-side organising metadata ONLY — deal tags are
-- never email content and never enter any Agent table. A deal groups mails across
-- accounts for a *read-only* aggregated timeline (F_G5); the account-isolation
-- boundary stays fully intact at the Agent layer.
--
-- Numbering: drafted as 010 in the card, but 010 was taken; next free is 013.
-- =============================================================================

CREATE TABLE IF NOT EXISTS deals (
    id          TEXT    NOT NULL PRIMARY KEY,  -- UUID v4
    name        TEXT    NOT NULL,
    color       TEXT    NOT NULL,              -- design-system colour token
    archived    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

-- Many-to-many join: one mail may belong to several deals, and a deal aggregates
-- mails from any account. ON DELETE CASCADE keeps the join clean when either side
-- is removed.
CREATE TABLE IF NOT EXISTS mail_deals (
    mail_id TEXT NOT NULL REFERENCES mails(id) ON DELETE CASCADE,
    deal_id TEXT NOT NULL REFERENCES deals(id) ON DELETE CASCADE,
    PRIMARY KEY (mail_id, deal_id)
);

CREATE INDEX IF NOT EXISTS idx_mail_deals_deal ON mail_deals(deal_id);
CREATE INDEX IF NOT EXISTS idx_mail_deals_mail ON mail_deals(mail_id);
