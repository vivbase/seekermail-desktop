-- 021_thread_summaries.sql
-- P-4 memory derived layer (analysis/54 §3.5): a precomputed one-line summary +
-- key entities per thread, written offline/idle and read at query time. This
-- turns "summarise everything" into "summarise a batch of summaries" — the
-- map-reduce that makes whole-mailbox analysis feasible without re-reading every
-- raw mail on each question.
--
-- One row per thread. `mail_count` / `latest_date` are snapshotted at summary
-- time so a grown or newer thread can be detected as stale and rebuilt; the
-- summary is otherwise read straight from here (no AI call on the read path).

CREATE TABLE IF NOT EXISTS thread_summaries (
    thread_id     TEXT    NOT NULL PRIMARY KEY REFERENCES threads(id) ON DELETE CASCADE,
    account_id    TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    summary       TEXT    NOT NULL,                 -- one-line gist of the thread
    key_entities  TEXT    NOT NULL DEFAULT '[]',    -- JSON array of short tag strings
    mail_count    INTEGER NOT NULL DEFAULT 0,       -- threads.mail_count when summarised
    latest_date   INTEGER NOT NULL DEFAULT 0,       -- threads.latest_date when summarised
    model         TEXT,                             -- LLM that produced the summary
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_thread_summaries_account_date
    ON thread_summaries(account_id, latest_date DESC);
