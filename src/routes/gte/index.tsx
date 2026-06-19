// GTE Knowledge Index (/gte) — prototype `page-gte` layout fully wired to the IPC
// surface: Account Breakdown from `list_accounts` (+ per-account `list_mails` count),
// engine status / total vectors from `get_gte_stats`, Top Topics from
// `get_topic_breakdown`, the recent-knowledge list from `list_knowledge_entries`, and
// the search box from `semantic_search`. Browser/dev renders via the IPC mock layer.
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";
import type { Account, SearchResult } from "@shared/bindings";

import { useAccounts } from "@/ipc/queries/accounts";
import { useGteStats, useKnowledgeEntries, useTopicBreakdown } from "@/ipc/queries/gte";
import { useMailCount } from "@/ipc/queries/mail";
import { MIN_SEARCH_LEN, useSemanticSearch } from "@/ipc/queries/search";
import PageBack from "@/components/layout/PageBack";

interface GteEntry {
  subject: string;
  date: string;
  excerpt: string;
  tags: string[];
}

function fmtDate(sec: number): string {
  const d = new Date(sec * 1000);
  const now = new Date();
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return `Today ${hm}`;
  return d.toLocaleDateString();
}

export default function Gte() {
  const { t } = useTranslation("gte");
  const navigate = useNavigate();
  const accounts = useAccounts();
  const statsQ = useGteStats();
  const s = statsQ.data;
  const topics = useTopicBreakdown().data ?? [];
  const topicMax = Math.max(1, ...topics.map((tp) => tp.count));
  const knowledge = useKnowledgeEntries();
  const browse: GteEntry[] = (knowledge.data ?? []).map((k) => ({
    subject: k.subject,
    date: fmtDate(k.dateSent),
    excerpt: k.excerpt,
    tags: k.tags,
  }));

  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const timer = useRef<ReturnType<typeof setTimeout>>();

  useEffect(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setDebounced(query.trim()), 300);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [query]);

  const q = query.trim();
  const live = debounced.length >= MIN_SEARCH_LEN;
  const search = useSemanticSearch({ query: debounced, enabled: live });

  // Seed entries filtered locally for short queries (responsive before the backend
  // minimum length); live semantic results once the query is long enough.
  const browseFiltered = q
    ? browse.filter(
        (e) =>
          e.subject.toLowerCase().includes(q.toLowerCase()) ||
          e.excerpt.toLowerCase().includes(q.toLowerCase()) ||
          e.tags.some((tag) => tag.toLowerCase().includes(q.toLowerCase())),
      )
    : browse;

  const liveEntries: GteEntry[] = (search.data?.items ?? []).map((r: SearchResult) => ({
    subject: r.subject,
    date: fmtDate(r.dateSent),
    excerpt: r.snippet,
    tags: [
      r.scoreLabel === "high"
        ? t("rel_high")
        : r.scoreLabel === "mid"
          ? t("rel_mid")
          : t("rel_low"),
    ],
  }));

  const entries = live ? liveEntries : browseFiltered;
  const loading = live ? search.isLoading : knowledge.isLoading;

  return (
    <div className="page active" style={{ height: "100%" }}>
      <div className="pg-header">
        <PageBack to="/" labelKey="back_to_dashboard" />
        <div className="pg-title">{t("title")}</div>
        <div className="pg-sub">{t("sub")}</div>
        <div className="pg-divider"></div>
      </div>

      <div className="pg-body">
        <div className="gte-status-row">
          <div className="gte-status-item">
            <span className="gte-pip" style={{ background: "var(--green)" }}></span>
            {t("status_engine")}
          </div>
          <div className="gte-status-item">
            <span className="gte-pip" style={{ background: "var(--green)" }}></span>
            {t("status_index", { version: s?.indexVersion ?? "—" })}
          </div>
          <div className="gte-status-item">
            <span className="gte-pip" style={{ background: "var(--amber)" }}></span>
            {t("status_syncing", { count: s?.accountsSyncing ?? 0 })}
          </div>
          <div
            style={{
              marginLeft: "auto",
              fontFamily: "var(--fm)",
              fontSize: 10,
              color: "var(--p7)",
            }}
          >
            {t("total_vectors", { n: (s?.vectorCount ?? 0).toLocaleString() })}
          </div>
        </div>

        <div className="gte-search-wrap">
          <svg className="search-icon" width="16" height="16" viewBox="0 0 16 16" fill="none">
            <circle cx="7" cy="7" r="4.5" stroke="#AEA89F" strokeWidth="1.2" />
            <path d="M10.5 10.5l3 3" stroke="#AEA89F" strokeWidth="1.2" strokeLinecap="round" />
          </svg>
          <input
            className="gte-search"
            placeholder={t("search_ph")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>

        <div className="gte-grid">
          <div className="gte-card">
            <div className="gte-card-title">{t("card_accounts")}</div>
            <div className="gte-acct-list">
              {(accounts.data ?? []).map((a) => (
                <GteAcctRow
                  key={a.id}
                  account={a}
                  onView={() => navigate(`/accounts/${a.id}/mail`)}
                  viewLabel={t("view")}
                />
              ))}
            </div>
          </div>

          <div className="gte-card">
            <div className="gte-card-title">{t("card_topics")}</div>
            <div className="gte-bar-wrap">
              {topics.map((tp) => (
                <div className="gte-bar-row" key={tp.label}>
                  <span className="gte-bar-label">{tp.label}</span>
                  <div className="gte-bar-track">
                    <div
                      className="gte-bar-fill"
                      style={{
                        width: `${Math.round((tp.count / topicMax) * 100)}%`,
                        background: `var(--${tp.color})`,
                      }}
                    ></div>
                  </div>
                  <span className="gte-bar-num">{tp.count}</span>
                </div>
              ))}
            </div>
          </div>
        </div>

        <div className="gte-topic-title">
          {q ? t("results_query", { query: q }) : t("results_recent")}
        </div>

        <div>
          {loading ? (
            <div aria-live="polite">
              {[0, 1, 2, 3].map((i) => (
                <div
                  key={i}
                  className="gte-entry"
                  style={{ opacity: 0.5 }}
                  aria-hidden={i > 0 ? true : undefined}
                >
                  <div
                    className="skel"
                    style={{ height: 13, width: "46%", borderRadius: 4, background: "var(--p4)" }}
                  ></div>
                  <div
                    className="skel"
                    style={{
                      height: 11,
                      width: "92%",
                      borderRadius: 4,
                      background: "var(--p4)",
                      marginTop: 10,
                    }}
                  ></div>
                  <div
                    className="skel"
                    style={{
                      height: 11,
                      width: "70%",
                      borderRadius: 4,
                      background: "var(--p4)",
                      marginTop: 6,
                    }}
                  ></div>
                </div>
              ))}
            </div>
          ) : entries.length ? (
            entries.map((e, i) => (
              <div className="gte-entry anim-in" key={e.subject + i}>
                <div className="gte-entry-top">
                  <div className="gte-entry-subject">{e.subject}</div>
                  <div className="gte-entry-meta">{e.date}</div>
                </div>
                <div className="gte-entry-excerpt">{e.excerpt}</div>
                <div className="gte-entry-tags">
                  {e.tags.map((tag) => (
                    <span className="gte-tag" key={tag}>
                      {tag}
                    </span>
                  ))}
                </div>
              </div>
            ))
          ) : (
            <div
              style={{
                padding: 32,
                textAlign: "center",
                fontFamily: "var(--fb)",
                fontStyle: "italic",
                color: "var(--p7)",
              }}
            >
              {t("no_results", { query: q })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** One account-breakdown row: chip + name/email + live indexed-mail count. */
function GteAcctRow({
  account,
  onView,
  viewLabel,
}: {
  account: Account;
  onView: () => void;
  viewLabel: string;
}) {
  const count = useMailCount({ accountId: account.id });
  const darkText = account.colorToken === "sage" || account.colorToken === "amber";
  return (
    <div className="gte-acct-row">
      <div
        className="gte-chip"
        style={{
          background: `var(--${account.colorToken})`,
          ...(darkText ? { color: "var(--p10)" } : {}),
        }}
      >
        {account.badgeLabel}
      </div>
      <div style={{ flex: 1 }}>
        <div className="gte-acct-name">{account.displayName}</div>
        <div className="gte-acct-email">{account.email}</div>
      </div>
      <div className="gte-acct-count" style={{ marginRight: 10 }}>
        {count.data != null ? count.data.toLocaleString() : "—"}
      </div>
      <button className="gte-more-btn" onClick={onView}>
        {viewLabel}
      </button>
    </div>
  );
}
