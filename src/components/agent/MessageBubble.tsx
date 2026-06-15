// One TEAM-channel message (T093, F_I2 §4). Three layouts by sender type:
//   • system → centered caption, no bubble
//   • human  → end-aligned dark bubble (the user)
//   • agent  → start-aligned light bubble with avatar + name chip
// Directional alignment uses logical margins (`ms-auto` / `me-auto`) so RTL
// locales mirror correctly. Every bubble is keyboard-focusable (Tab) per F_I2 §6.

import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import { parseMessageText, type ImMessage } from "@/ipc/im";
import AgentAvatar from "./AgentAvatar";
import AgentNameChip from "./AgentNameChip";
import QueryCardEmbed from "./QueryCardEmbed";

interface MessageBubbleProps {
  message: ImMessage;
  /** Resolved sender account when `senderType === "agent"`. */
  account?: Account;
}

export default function MessageBubble({ message, account }: MessageBubbleProps) {
  const { t } = useTranslation("team");

  if (message.senderType === "system") {
    return (
      <div className="my-2 flex justify-center">
        <p tabIndex={0} className="max-w-[80%] text-balance text-center font-ui text-xs text-p7">
          {parseMessageText(message.content)}
        </p>
      </div>
    );
  }

  if (message.senderType === "human") {
    return (
      <div className="flex">
        <div
          tabIndex={0}
          aria-label={t("team_member_you")}
          className="ms-auto max-w-[75%] rounded-card bg-p9 px-3 py-2 text-white"
        >
          <p className="whitespace-pre-wrap font-body text-sm">
            {parseMessageText(message.content)}
          </p>
        </div>
      </div>
    );
  }

  // Agent message (left/start aligned).
  return (
    <div className="flex">
      <div tabIndex={0} className="me-auto flex max-w-[75%] gap-2">
        {account ? (
          <AgentAvatar
            email={account.email}
            colorToken={account.colorToken}
            size={32}
            className="mt-0.5 shrink-0"
          />
        ) : (
          <span className="mt-0.5 h-8 w-8 shrink-0 rounded-avatar bg-p5" aria-hidden />
        )}
        <div className="min-w-0">
          <div className="mb-0.5">
            {account ? (
              <AgentNameChip
                displayName={account.displayName}
                email={account.email}
                isPrimary={account.isPrimary}
              />
            ) : (
              <span className="font-body text-sm text-p10">{message.senderId}</span>
            )}
          </div>
          {message.messageType === "query_card" ? (
            <QueryCardEmbed message={message} />
          ) : (
            <div className="rounded-card border border-divider bg-surface px-3 py-2">
              <p className="whitespace-pre-wrap font-body text-sm text-p10">
                {parseMessageText(message.content)}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
