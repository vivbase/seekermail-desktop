// TEAM channel container (T093, F_I2 §4). Wires the topbar, the message stream
// (with per-day dividers + auto-scroll to bottom), the composer, and the member
// drawer. All members — the human user and every AI agent — see this one shared
// channel (no private chats). An all-offline banner surfaces when every agent's
// presence is "offline".
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { useAccounts } from "@/ipc/queries/accounts";
import { useAgentStatuses } from "@/ipc/queries/agents";
import { useImMessages } from "@/ipc/queries/im";
import { usePendingCounts } from "@/ipc/queries/drafts";
import type { AgentStatusValue } from "@/ipc/agents";
import type { ImMessage } from "@/ipc/im";
import ChannelTopbar from "./ChannelTopbar";
import MessageBubble from "./MessageBubble";
import ChannelInput from "./ChannelInput";
import MemberDrawer from "./MemberDrawer";

/** Calendar-day key (local time) for grouping the stream into dated sections. */
function dayKey(ts: number): string {
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

type Row =
  | { kind: "divider"; key: string; label: string }
  | { kind: "message"; message: ImMessage };

export default function TeamChannel() {
  const { t } = useTranslation("team");
  const { data: accounts = [] } = useAccounts();
  const { data: statuses = [] } = useAgentStatuses();
  const { data: page } = useImMessages();
  const messages = useMemo(() => page?.items ?? [], [page]);

  const [membersOpen, setMembersOpen] = useState(false);
  const streamRef = useRef<HTMLDivElement>(null);

  const accountsById = useMemo(
    () => Object.fromEntries(accounts.map((a) => [a.id, a])),
    [accounts],
  );
  const statusById = useMemo(
    () => Object.fromEntries(statuses.map((s) => [s.accountId, s.status as AgentStatusValue])),
    [statuses],
  );
  const primary = accounts.find((a) => a.isPrimary);

  const allOffline =
    accounts.length > 0 && statuses.length > 0 && statuses.every((s) => s.status === "offline");

  // "+ New Query" focuses the composer; the digest tallies open decisions + drafts.
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const { draftCount } = usePendingCounts();
  const queryMsgs = messages.filter((m) => m.messageType === "query_card" && m.status === "pending");
  const decisions = queryMsgs.length;
  const high = queryMsgs.filter((m) => {
    try {
      const v = JSON.parse(m.content) as Record<string, unknown>;
      return (v.triggerType ?? v.trigger_type) === "T4";
    } catch {
      return false;
    }
  }).length;

  // Lock the stream to the bottom whenever a new message arrives.
  useEffect(() => {
    const el = streamRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length]);

  const todayKey = dayKey(Math.floor(Date.now() / 1000));
  const rows: Row[] = [];
  let lastDay = "";
  for (const message of messages) {
    const key = dayKey(message.createdAt);
    if (key !== lastDay) {
      const label =
        key === todayKey
          ? t("team_today")
          : new Date(message.createdAt * 1000).toLocaleDateString(undefined, {
              month: "short",
              day: "numeric",
              year: "numeric",
            });
      rows.push({ kind: "divider", key: `divider-${key}`, label });
      lastDay = key;
    }
    rows.push({ kind: "message", message });
  }

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <ChannelTopbar
        accounts={accounts}
        statusById={statusById}
        agentCount={accounts.length}
        pendingCount={decisions}
        membersOpen={membersOpen}
        onToggleMembers={() => setMembersOpen((o) => !o)}
        onNewQuery={() => composerRef.current?.focus()}
      />

      {allOffline && (
        <div role="status" className="bg-amber px-5 py-2 text-center font-ui text-xs text-p10">
          {t("team_all_offline_banner")}
        </div>
      )}

      {/* Today digest (USER_JOURNEYS §4) — grouped attention, pinned above the scroll. */}
      {accounts.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 border-b border-divider bg-p2 px-5 py-2">
          <span className="shrink-0 rounded-chip bg-p9 px-2 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-wider text-white">
            {t("team_digest_pin")}
          </span>
          <span className="min-w-0 font-body text-xs text-p8">
            {t("team_digest_summary", {
              agents: accounts.length,
              decisions,
              drafts: draftCount,
              high,
            })}
          </span>
          <Link
            to="/pending"
            className="ms-auto shrink-0 rounded-chip border border-divider px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider text-p9 hover:bg-p4"
          >
            {t("team_digest_review")}
          </Link>
        </div>
      )}

      <div
        ref={streamRef}
        role="log"
        aria-label={t("team_channel_name")}
        aria-live="polite"
        className="min-h-0 flex-1 overflow-y-auto px-5 py-4"
      >
        {rows.map((row) =>
          row.kind === "divider" ? (
            <div key={row.key} className="my-3 flex items-center gap-3">
              <span className="h-px flex-1 bg-p5" />
              <span className="font-ui text-[10px] uppercase tracking-wider text-p7">
                {row.label}
              </span>
              <span className="h-px flex-1 bg-p5" />
            </div>
          ) : (
            <div key={row.message.id} className="mb-2">
              <MessageBubble
                message={row.message}
                account={
                  row.message.senderType === "agent"
                    ? accountsById[row.message.senderId]
                    : undefined
                }
              />
            </div>
          ),
        )}
      </div>

      <ChannelInput accounts={accounts} disabled={accounts.length === 0} inputRef={composerRef} />

      <MemberDrawer
        open={membersOpen}
        onClose={() => setMembersOpen(false)}
        accounts={accounts}
        statusById={statusById}
        primary={primary}
      />
    </div>
  );
}
