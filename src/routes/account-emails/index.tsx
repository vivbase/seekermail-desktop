// Per-account email list (/accounts/:id/mail) — prototype `page-acct-emails` layout
// wired to the real IPC surface. The account header comes from `list_accounts`; the
// list from `list_mails` (paginated, by accountId); the expanded body from `get_mail`;
// totals from count queries. Filters (All / Unread / Today / Risk Flagged), search,
// and sort run over the loaded page. Browser/dev still renders via the IPC mock layer
// (src/ipc/client.ts), so the same code path works with and without a real backend.
import { useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import type { MailSummary } from "@shared/bindings";

import { showToast } from "@/components/ui/Toast";
import { useAccounts } from "@/ipc/queries/accounts";
import { useMailCount, useMailDetail, useMailsInfinite } from "@/ipc/queries/mail";
import { cn } from "@/lib/cn";

type AeFilter = "all" | "unread" | "today" | "flagged";
type AeSort = "new" | "old" | "sender";

const RISK_RE = /(risk|anomaly|mismatch|flag|fraud|phish|⚠)/i;

/** Sage/amber chips sit on light backgrounds and need dark text (design tokens). */
function chipTextColor(colorToken: string): string | undefined {
  return colorToken === "sage" || colorToken === "amber" ? "var(--p10)" : undefined;
}

function startOfTodaySecs(): number {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}

function fmtTime(sec: number): string {
  const d = new Date(sec * 1000);
  const now = new Date();
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return `Today ${hm}`;
  const yest = new Date(now);
  yest.setDate(now.getDate() - 1);
  if (d.toDateString() === yest.toDateString()) return `Yesterday ${hm}`;
  return d.toLocaleDateString();
}

export default function AccountEmails() {
  const { t } = useTranslation("accountEmails");
  const navigate = useNavigate();
  const { id = "" } = useParams();

  const accounts = useAccounts();
  const account = accounts.data?.find((a) => a.id === id);

  const total = useMailCount({ accountId: id });
  const unread = useMailCount({ accountId: id, isUnread: true });
  const mailsQuery = useMailsInfinite({ accountId: id });

  const [filter, setFilter] = useState<AeFilter>("all");
  const [sort, setSort] = useState<AeSort>("new");
  const [query, setQuery] = useState("");
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const allMails: MailSummary[] = useMemo(
    () => mailsQuery.data?.pages.flatMap((p) => p.items) ?? [],
    [mailsQuery.data],
  );

  const sot = startOfTodaySecs();
  const todayCount = useMemo(() => allMails.filter((m) => m.dateSent >= sot).length, [allMails, sot]);

  const emails = useMemo(() => {
    let list = allMails.slice();
    if (filter === "unread") list = list.filter((m) => !m.isRead);
    else if (filter === "today") list = list.filter((m) => m.dateSent >= sot);
    else if (filter === "flagged")
      list = list.filter((m) => RISK_RE.test((m.subject ?? "") + " " + (m.snippet ?? "")));
    const q = query.trim().toLowerCase();
    if (q)
      list = list.filter((m) =>
        ((m.fromName ?? "") + " " + m.fromEmail + " " + m.subject + " " + (m.snippet ?? ""))
          .toLowerCase()
          .includes(q),
      );
    list.sort((a, b) => {
      if (sort === "old") return a.dateSent - b.dateSent;
      if (sort === "sender") return (a.fromName ?? a.fromEmail).localeCompare(b.fromName ?? b.fromEmail);
      return b.dateSent - a.dateSent;
    });
    return list;
  }, [allMails, filter, sort, query, sot]);

  if (!account && !accounts.isLoading) {
    return (
      <div className="page active" style={{ height: "100%" }}>
        <div className="pg-header">
          <button className="pg-back" onClick={() => navigate("/agents")}>
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <path d="M9 2L4 7l5 5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
            {t("back")}
          </button>
          <div className="pg-title">{t("not_found")}</div>
          <div className="pg-divider"></div>
        </div>
      </div>
    );
  }

  const chip = account?.badgeLabel ?? "·";
  const color = account ? `var(--${account.colorToken})` : "var(--p9)";
  const tc = account ? chipTextColor(account.colorToken) : undefined;
  const name = account?.displayName ?? "";
  const email = account?.email ?? "";
  const FILTERS: AeFilter[] = ["all", "unread", "today", "flagged"];

  return (
    <div className="page active" style={{ height: "100%" }}>
      <div className="pg-header">
        <button className="pg-back" onClick={() => navigate("/agents")}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <path d="M9 2L4 7l5 5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
          {t("back")}
        </button>
        <div className="pg-title">{t("title", { name })}</div>
        <div className="pg-sub">{t("sub", { email, total: (total.data ?? 0).toLocaleString() })}</div>
        <div className="pg-divider"></div>
      </div>

      <div className="pg-body">
        <div className="ae-hero">
          <div className="ae-av" style={{ background: color, color: tc || "white" }}>
            {chip}
          </div>
          <div>
            <div className="ae-name">{name}</div>
            <div className="ae-email">{email}</div>
          </div>
          <div className="ae-stats">
            <div style={{ textAlign: "right" }}>
              <div className="ae-stat-n">{(total.data ?? 0).toLocaleString()}</div>
              <div className="ae-stat-l">{t("stat_total")}</div>
            </div>
            <div style={{ textAlign: "right" }}>
              <div className="ae-stat-n" style={{ color: "var(--red)" }}>
                {unread.data ?? 0}
              </div>
              <div className="ae-stat-l">{t("stat_unread")}</div>
            </div>
            <div style={{ textAlign: "right" }}>
              <div className="ae-stat-n">{todayCount}</div>
              <div className="ae-stat-l">{t("stat_today")}</div>
            </div>
          </div>
        </div>

        <div className="ae-filter-row">
          <span className="ae-filter-lbl">{t("filter")}</span>
          {FILTERS.map((f) => (
            <button key={f} className={cn("chip", filter === f && "active")} onClick={() => setFilter(f)}>
              {t(`chip_${f}`)}
            </button>
          ))}
          <div className="bulk-spacer"></div>
          <div className="ut-search-wrap">
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
              <circle cx="6" cy="6" r="4" stroke="#AEA89F" strokeWidth="1.3" />
              <path d="M9.5 9.5l3 3" stroke="#AEA89F" strokeWidth="1.3" strokeLinecap="round" />
            </svg>
            <input
              className="ut-search"
              placeholder={t("search_ph")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <select
            className="ut-sort"
            value={sort}
            onChange={(e) => setSort(e.target.value as AeSort)}
            aria-label="Sort order"
          >
            <option value="new">{t("sort_new")}</option>
            <option value="old">{t("sort_old")}</option>
            <option value="sender">{t("sort_sender")}</option>
          </select>
        </div>

        <div>
          {mailsQuery.isLoading ? (
            <div style={{ padding: 40, textAlign: "center", fontFamily: "var(--fb)", fontStyle: "italic", color: "var(--p7)" }}>
              {t("loading")}
            </div>
          ) : emails.length === 0 ? (
            <div style={{ padding: 40, textAlign: "center", fontFamily: "var(--fb)", fontStyle: "italic", color: "var(--p7)" }}>
              {t("empty")}
            </div>
          ) : (
            emails.map((m, i) => (
              <EmailRow
                key={m.id}
                mail={m}
                index={i}
                chip={chip}
                color={color}
                tc={tc}
                flagged={RISK_RE.test((m.subject ?? "") + " " + (m.snippet ?? ""))}
                expanded={expandedId === m.id}
                onToggle={() => setExpandedId((cur) => (cur === m.id ? null : m.id))}
                t={t}
              />
            ))
          )}
        </div>
      </div>
    </div>
  );
}

type TFn = (key: string, opts?: Record<string, unknown>) => string;

function EmailRow({
  mail,
  index,
  chip,
  color,
  tc,
  flagged,
  expanded,
  onToggle,
  t,
}: {
  mail: MailSummary;
  index: number;
  chip: string;
  color: string;
  tc: string | undefined;
  flagged: boolean;
  expanded: boolean;
  onToggle: () => void;
  t: TFn;
}) {
  // Full body is fetched lazily (get_mail) only while the row is open.
  const detail = useMailDetail(expanded ? mail.id : null);
  const bodyText = detail.data?.bodyText ?? detail.data?.bodyHtml?.replace(/<[^>]+>/g, " ") ?? mail.snippet ?? "";

  return (
    <div
      className={cn("email-row", !mail.isRead && "unread")}
      style={{ animationDelay: `${index * 20}ms` }}
      onClick={onToggle}
    >
      <div className="e-av" style={{ background: color, color: tc || "white" }}>
        {chip}
      </div>
      <div className="e-main">
        <div className="e-top-row">
          <span className="e-from">{mail.fromName ?? mail.fromEmail}</span>
          <span className="e-time">{fmtTime(mail.dateSent)}</span>
        </div>
        <div className="e-subject">
          {mail.subject}
          {flagged && <span className="e-risk-tag">⚠ {t("risk_lbl")}</span>}
        </div>
        <div className="e-preview">{mail.snippet}</div>
        {mail.hasAttachments && (
          <div className="e-badges">
            <span className="e-badge badge-terra">{t("badge_attachment")}</span>
          </div>
        )}
      </div>
      {expanded && (
        <div className="e-expand" style={{ display: "block" }}>
          <div className="e-expanded-body" style={{ whiteSpace: "pre-line" }}>
            {detail.isLoading ? t("loading") : bodyText}
          </div>
          <div className="e-expanded-actions">
            <button
              className="e-btn"
              onClick={(ev) => {
                ev.stopPropagation();
                showToast(t("toast_reply", { name: mail.fromName ?? mail.fromEmail }));
              }}
            >
              {t("reply")}
            </button>
            <button
              className="e-btn"
              onClick={(ev) => {
                ev.stopPropagation();
                showToast(t("toast_forward"));
              }}
            >
              {t("forward")}
            </button>
            <button
              className="e-btn e-btn-ghost"
              onClick={(ev) => {
                ev.stopPropagation();
                showToast(t("toast_mark_read"));
              }}
            >
              {t("mark_read")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
