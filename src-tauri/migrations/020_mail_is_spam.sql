-- 020_mail_is_spam.sql
-- Local "marked spam" marker, the Junk analogue of `is_deleted` (the Trash
-- marker). When the user marks a message spam we set `is_spam = 1` immediately so
-- the Spam tab surfaces it at once — without waiting for the server-side MOVE to
-- Junk to drain and the Junk folder to re-sync. The row keeps its origin folder
-- until that move-detection re-points it to JUNK (mirrors how a soft-deleted mail
-- keeps INBOX until the Trash move syncs). The Spam view is therefore
-- `folder = 'JUNK' OR is_spam = 1`; every other view excludes `is_spam = 1`.
ALTER TABLE mails ADD COLUMN is_spam INTEGER NOT NULL DEFAULT 0;
