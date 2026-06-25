-- 022_inbox_digest.sql
-- P-4 memory derived layer (analysis/54 §3.5 + §3.3): a rolling, precomputed
-- inbox overview — the *level-2* reduction of the per-thread summaries
-- (021_thread_summaries). One short paragraph per account, written offline and
-- read instantly, so a large mailbox's "summarise everything" never has to pack
-- every thread summary: the digest is the summary-of-summaries the budget packer
-- falls back to.
--
-- One current digest per account (the rolling cache). `thread_count` records how
-- many summaries were reduced into it; `generated_at` lets callers treat an old
-- digest as stale.

CREATE TABLE IF NOT EXISTS inbox_digest (
    account_id    TEXT    NOT NULL PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    digest        TEXT    NOT NULL,                 -- 2–4 sentence inbox overview
    thread_count  INTEGER NOT NULL DEFAULT 0,       -- summaries reduced into this digest
    unread_count  INTEGER NOT NULL DEFAULT 0,       -- unread inbox mail at digest time
    model         TEXT,                             -- LLM that produced the digest
    generated_at  INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
