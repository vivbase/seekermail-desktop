-- =============================================================================
-- SeekerMail · SQLite Schema · Migration 014 · Drop deal tags (G5 removed)
-- Forward-only, additive. The G5 "transaction view" (deals) feature was removed
-- from the product, so its human-side organising tables are dropped here. No
-- Agent or mail content is affected — deal tags were always isolated, read-only
-- metadata. Migration 013 (which created these tables) is left intact as an
-- applied-ledger entry per the forward-only policy; this migration is the
-- authoritative removal, so a fresh database ends up with no deal tables.
-- =============================================================================

DROP INDEX IF EXISTS idx_mail_deals_mail;
DROP INDEX IF EXISTS idx_mail_deals_deal;
DROP TABLE IF EXISTS mail_deals;
DROP TABLE IF EXISTS deals;
