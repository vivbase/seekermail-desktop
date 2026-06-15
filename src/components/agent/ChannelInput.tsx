// TEAM channel composer (T093, F_I2 §4). Plain-text input with an `@` mention
// picker (Tab/Arrow to cycle, Enter to select), Enter / Cmd+Enter to send, and a
// failed-send retry strip. Mentions are inserted as literal `@Name` text — the
// backend broadcasts the message verbatim (no server-side mention parsing in v0.5).
import { useRef, useState, type KeyboardEvent, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import { AlertCircle } from "lucide-react";
import type { Account } from "@shared/bindings";

import { usePostImMessage } from "@/ipc/queries/im";
import { textContent } from "@/ipc/im";
import { cn } from "@/lib/cn";
import AgentAvatar from "./AgentAvatar";

interface ChannelInputProps {
  accounts: Account[];
  disabled?: boolean;
  /** Optional external ref to the textarea so "+ New Query" can focus the composer. */
  inputRef?: RefObject<HTMLTextAreaElement>;
}

export default function ChannelInput({ accounts, disabled = false, inputRef }: ChannelInputProps) {
  const { t } = useTranslation("team");
  const post = usePostImMessage();
  const localRef = useRef<HTMLTextAreaElement>(null);
  const taRef = inputRef ?? localRef;

  const [text, setText] = useState("");
  const [failed, setFailed] = useState<string[]>([]);
  const [mentionOpen, setMentionOpen] = useState(false);
  const [mentionIdx, setMentionIdx] = useState(0);

  // The `@token` currently being typed at the caret end, if any.
  const mentionMatch = /@(\w*)$/.exec(text);
  const mentionQuery = mentionMatch ? (mentionMatch[1] ?? "").toLowerCase() : null;
  const mentionMatches =
    mentionQuery === null
      ? []
      : accounts.filter((a) => a.displayName.toLowerCase().includes(mentionQuery));
  const showMention = mentionOpen && mentionMatches.length > 0;

  const onChange = (value: string) => {
    setText(value);
    setMentionOpen(/@(\w*)$/.test(value));
    setMentionIdx(0);
  };

  const selectMention = (account: Account) => {
    setText((cur) => cur.replace(/@(\w*)$/, `@${account.displayName} `));
    setMentionOpen(false);
    taRef.current?.focus();
  };

  const send = (content: string) => {
    post.mutate(
      {
        senderType: "human",
        senderId: "human",
        messageType: "text",
        content: textContent(content),
      },
      { onError: () => setFailed((f) => [...f, content]) },
    );
  };

  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed || disabled) return;
    send(trimmed);
    setText("");
    setMentionOpen(false);
  };

  const retry = (content: string) => {
    setFailed((f) => f.filter((c) => c !== content));
    send(content);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (showMention) {
      if (e.key === "ArrowDown" || e.key === "Tab") {
        e.preventDefault();
        setMentionIdx((i) => (i + 1) % mentionMatches.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setMentionIdx((i) => (i - 1 + mentionMatches.length) % mentionMatches.length);
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const acc = mentionMatches[mentionIdx];
        if (acc) selectMention(acc);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setMentionOpen(false);
        return;
      }
    }
    // Enter (or Cmd/Ctrl+Enter) sends; Shift+Enter inserts a newline.
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className="relative border-t border-divider bg-surface p-3">
      {failed.length > 0 && (
        <ul className="mb-2 flex flex-col gap-1">
          {failed.map((content, i) => (
            <li
              key={`${content}-${i}`}
              className="flex items-center justify-between gap-2 rounded-chip bg-p4 px-2 py-1"
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <AlertCircle size={14} className="shrink-0 text-red" aria-hidden />
                <span className="truncate font-body text-xs text-p9">{content}</span>
              </span>
              <span className="flex shrink-0 items-center gap-2">
                <span className="font-ui text-[10px] uppercase tracking-wider text-red">
                  {t("team_message_failed")}
                </span>
                <button
                  type="button"
                  onClick={() => retry(content)}
                  className="rounded-chip border border-divider px-2 py-0.5 font-ui text-[10px] uppercase tracking-wider text-p9 hover:bg-p3"
                >
                  {t("team_retry")}
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}

      {showMention && (
        <ul
          role="listbox"
          aria-label={t("team_mention_hint")}
          className="absolute bottom-full mb-1 max-h-48 w-64 overflow-y-auto rounded-card border border-divider bg-surface shadow-card"
        >
          {mentionMatches.map((a, i) => (
            <li key={a.id}>
              <button
                type="button"
                role="option"
                aria-selected={i === mentionIdx}
                onClick={() => selectMention(a)}
                className={cn(
                  "flex w-full items-center gap-2 px-2 py-1.5 text-start",
                  i === mentionIdx ? "bg-p4" : "hover:bg-p3",
                )}
              >
                <AgentAvatar email={a.email} colorToken={a.colorToken} size={20} />
                <span className="truncate font-body text-sm text-p10">{a.displayName}</span>
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="flex items-end gap-2">
        <textarea
          ref={taRef}
          value={text}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          rows={1}
          aria-label={t("team_input_placeholder")}
          placeholder={disabled ? t("team_input_disabled") : t("team_input_placeholder")}
          className="min-h-[2.25rem] flex-1 resize-none rounded-chip border border-divider bg-p1 px-3 py-2 font-body text-sm text-p10 placeholder:text-p7 disabled:opacity-50"
        />
        <button
          type="button"
          onClick={submit}
          disabled={disabled || !text.trim()}
          className="shrink-0 rounded-chip bg-p9 px-4 py-2 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {t("team_send")}
        </button>
      </div>
      <p className="mt-1 font-ui text-[10px] text-p7">{t("team_mention_hint")}</p>
    </div>
  );
}
