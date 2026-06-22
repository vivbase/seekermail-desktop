// All-mail unified inbox (T048). Cross-account view backed by ThreadList with
// accountId=null (all accounts). Provides an account-filter control so the user
// can narrow to a single account without leaving the route.
// selectedAccountId is kept in local state here (not the global selection store)
// to avoid interfering with the L0 dashboard selection.
// Focus lands on <h1> on mount (dev/11 §3).
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Account } from "@shared/bindings";

import { ThreadList } from "@/components/mail/ThreadList";
import { useAccounts } from "@/ipc/queries/accounts";
import { useMailCount } from "@/ipc/queries/mail";
import { cn } from "@/lib/cn";

/** All Mail folder tabs (prototype: Inbox · All Mail · Sent · Drafts · Trash).
 * `inbox` is the received-only IMAP INBOX folder and is the default landing tab
 * (IA: "main view defaults to Inbox"). `all` applies no folder filter, so it
 * unifies received + sent + archived across the account(s). */
type FolderView = "inbox" | "all" | "sent" | "drafts" | "spam" | "trash";
const FOLDER_MAP: Record<FolderView, string | null> = {
  inbox: "INBOX",
  all: null,
  sent: "SENT",
  drafts: "DRAFTS",
  spam: "JUNK",
  trash: "TRASH",
};

export default function AllMail() {
  const { t } = useTranslation(["settings", "list"]);
  const headingRef = useRef<HTMLHeadingElement>(null);

  // Local account filter — null means all accounts.
  const [filterAccountId, setFilterAccountId] = useState<string | null>(null);
  // Mailbox folder tab — defaults to the received-only Inbox (IA default view).
  const [folderView, setFolderView] = useState<FolderView>("inbox");

  const { data: accounts } = useAccounts();

  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  return (
    <div className="flex h-full flex-col">
      {/* Page header */}
      <header className="shrink-0 border-b border-divider px-6 py-5">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <h1
            ref={headingRef}
            tabIndex={-1}
            className="font-display text-3xl italic text-p10 outline-none"
          >
            {t("list_page_inbox")}
          </h1>

          {/* Account filter */}
          <AccountFilter
            accounts={accounts ?? []}
            value={filterAccountId}
            onChange={setFilterAccountId}
            allLabel={t("list_lbl_all_accounts")}
            ariaLabel={t("list_lbl_account_filter")}
          />
        </div>

        {/* Subtitle metadata (prototype: "across N accounts · sorted by received time"). */}
        <p className="mt-1 font-ui text-[11px] uppercase tracking-[0.06em] text-p8">
          {t("list:all_mail_subtitle", { count: accounts?.length ?? 0 })}
        </p>

        {/* Account chips (quick-select) */}
        {(accounts?.length ?? 0) > 1 && (
          <div className="mt-3 flex flex-wrap gap-2">
            <AccountChip
              active={filterAccountId === null}
              label={t("list_lbl_all_accounts")}
              colorToken="p9"
              onSelect={() => setFilterAccountId(null)}
            />
            {(accounts ?? []).map((acct) => (
              <AccountChip
                key={acct.id}
                active={filterAccountId === acct.id}
                label={acct.displayName}
                colorToken={acct.colorToken}
                onSelect={() => setFilterAccountId(acct.id)}
              />
            ))}
          </div>
        )}
      </header>

      {/* Folder tabs (prototype am-tab row): Inbox · All Mail · Sent · Drafts · Trash */}
      <FolderTabs value={folderView} onChange={setFolderView} accountId={filterAccountId} />

      {/* Thread list — ThreadList reads mailFilter from the store; accountId
          prop filters at the data layer (accountId=null = all accounts). */}
      <div className="min-h-0 flex-1 overflow-hidden">
        <ThreadList accountId={filterAccountId} folder={FOLDER_MAP[folderView]} />
      </div>
    </div>
  );
}

// ── Account filter select ─────────────────────────────────────────────────────

function AccountFilter({
  accounts,
  value,
  onChange,
  allLabel,
  ariaLabel,
}: {
  accounts: Account[];
  value: string | null;
  onChange: (id: string | null) => void;
  allLabel: string;
  ariaLabel: string;
}) {
  if (accounts.length <= 1) return null;

  return (
    <select
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value || null)}
      aria-label={ariaLabel}
      className="rounded-chip border border-divider bg-surface px-3 py-1.5 font-ui text-xs text-p9 focus:outline focus:outline-2 focus:outline-p9"
    >
      <option value="">{allLabel}</option>
      {accounts.map((acct) => (
        <option key={acct.id} value={acct.id}>
          {acct.displayName}
        </option>
      ))}
    </select>
  );
}

// ── Account chip (quick-select pill) ─────────────────────────────────────────

function AccountChip({
  active,
  label,
  colorToken,
  onSelect,
}: {
  active: boolean;
  label: string;
  colorToken: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={active}
      className={`rounded-chip border px-3 py-1 font-ui text-xs transition-colors ${
        active
          ? "border-transparent bg-p9 text-white"
          : "border-divider bg-surface text-p9 hover:bg-p4"
      }`}
    >
      {/* Color dot */}
      <span
        className={`me-1.5 inline-block h-1.5 w-1.5 rounded-avatar bg-${colorToken}`}
        aria-hidden
      />
      {label}
    </button>
  );
}

// ── Folder tabs (Inbox · All Mail · Sent · Drafts · Trash) ───────────────────

function FolderTabs({
  value,
  onChange,
  accountId,
}: {
  value: FolderView;
  onChange: (v: FolderView) => void;
  accountId: string | null;
}) {
  const { t } = useTranslation("list");
  const acct = accountId ?? undefined;
  // Per-tab counts, filtered at the data layer by folder.
  const inboxCount = useMailCount({ accountId: acct, folder: "INBOX" }).data ?? 0;
  const allCount = useMailCount({ accountId: acct }).data ?? 0;
  const sentCount = useMailCount({ accountId: acct, folder: "SENT" }).data ?? 0;
  const draftCount = useMailCount({ accountId: acct, folder: "DRAFTS" }).data ?? 0;
  const spamCount = useMailCount({ accountId: acct, folder: "JUNK" }).data ?? 0;
  const trashCount = useMailCount({ accountId: acct, folder: "TRASH" }).data ?? 0;

  const tabs: { key: FolderView; label: string; count: number }[] = [
    { key: "inbox", label: t("tab_inbox"), count: inboxCount },
    { key: "all", label: t("tab_all_mail"), count: allCount },
    { key: "sent", label: t("tab_sent"), count: sentCount },
    { key: "drafts", label: t("tab_drafts"), count: draftCount },
    { key: "spam", label: t("tab_spam"), count: spamCount },
    { key: "trash", label: t("tab_trash"), count: trashCount },
  ];

  return (
    <div
      role="tablist"
      aria-label={t("folders_title")}
      className="flex items-center gap-1 border-b border-divider px-6"
    >
      {tabs.map((tab) => {
        const active = value === tab.key;
        return (
          <button
            key={tab.key}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(tab.key)}
            className={cn(
              "-mb-px flex items-center gap-1 whitespace-nowrap border-b-2 px-3.5 py-2.5 font-ui text-[10px] font-semibold uppercase tracking-[0.07em] transition-colors",
              "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
              active ? "border-p10 text-p10" : "border-transparent text-p8 hover:text-p9",
            )}
          >
            {tab.label}
            <span
              className={cn(
                "rounded-[10px] px-1.5 py-px font-mono text-[8px] leading-none",
                active ? "bg-p10 text-white" : "bg-p4 text-p8",
              )}
            >
              {tab.count}
            </span>
          </button>
        );
      })}
    </div>
  );
}
