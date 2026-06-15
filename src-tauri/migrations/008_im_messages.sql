-- =============================================================================
-- 008_im_messages.sql — Agent-IM (TEAM) channel message store (T092, F_I2 §5)
--
-- Forward-only (08 §migrations): never edits 001–007. The card was authored
-- against an earlier tree where this was "004"; migrations 004–007 already exist,
-- so it lands as 008.
--
-- The TEAM channel is a single shared group chat: there are NO private channels.
-- `channel_id` is pinned to 'main' by a CHECK constraint — the data-layer
-- guarantee behind the "no private chats" invariant (root CLAUDE.md "Agent-IM").
-- =============================================================================

CREATE TABLE IF NOT EXISTS im_messages (
    id              TEXT    NOT NULL PRIMARY KEY,   -- UUID v4
    channel_id      TEXT    NOT NULL CHECK(channel_id = 'main'),
    sender_type     TEXT    NOT NULL CHECK(sender_type IN ('human','agent','system')),
    sender_id       TEXT    NOT NULL,               -- account id, or 'system'/'human'
    message_type    TEXT    NOT NULL CHECK(message_type IN ('text','query_card','card_reply','status')),
    content         TEXT    NOT NULL,               -- JSON payload (text / QA card / status)
    linked_email_id TEXT    REFERENCES mails(id) ON DELETE SET NULL,
    status          TEXT    NOT NULL DEFAULT 'resolved'
                        CHECK(status IN ('pending','answered','skipped','resolved')),
    created_at      INTEGER NOT NULL,
    read_at         INTEGER
);

-- Primary read path: the channel timeline in created order (T093 message stream).
CREATE INDEX IF NOT EXISTS idx_im_channel_created
    ON im_messages(channel_id, created_at ASC);

-- Per-sender history (Agent presence / filtering by account, T094).
CREATE INDEX IF NOT EXISTS idx_im_sender
    ON im_messages(sender_id, created_at DESC);

-- Open query cards awaiting a human answer (I3/I4 surfaces).
CREATE INDEX IF NOT EXISTS idx_im_pending
    ON im_messages(status, created_at)
    WHERE status = 'pending';
