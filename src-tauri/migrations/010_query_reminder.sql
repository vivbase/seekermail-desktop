-- =============================================================================
-- 010_query_reminder.sql — T4 reminder bookkeeping (T097)
--
-- Forward-only (08 §migrations). The card called this "006"; that number is
-- taken, so it lands as 010. T4 risk queries never expire (F_I3 §4.2) but get a
-- merged daily reminder (the F5 "pressure-relief valve"); this column records
-- when the last reminder for a query was posted so we don't re-nag within a day.
-- =============================================================================

ALTER TABLE pending_queries ADD COLUMN last_reminder_at INTEGER;
