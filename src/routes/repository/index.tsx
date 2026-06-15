// Repository / Knowledge Index (/repository) — 1:1 port of the prototype `page-repo`
// markup and behaviour (UI/seekermail-unified.html). Uses the ported prototype CSS
// classes (search-hero / c3-drawer / k-card / repo-sidebar / gte-panel) so it renders
// identically. The prototype's vanilla-JS handlers become React state: semantic search
// with mode detection, account/topic/date filters, sort, expandable knowledge cards,
// an advanced filter drawer with saved searches, and a live reindex progress animation.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type { Account, AiDecisionRow, SavedSearch, SearchResult } from "@shared/bindings";
import type { KnowledgeEntry } from "@/ipc/gteStats";

import ConfirmDialog from "@/components/ui/ConfirmDialog";
import { showToast } from "@/components/ui/Toast";
import { useAccounts } from "@/ipc/queries/accounts";
import { EMPTY_AUDIT_FILTERS, useAiDecisions } from "@/ipc/queries/audit";
import { useGteStats, useKnowledgeEntries, useTopicBreakdown } from "@/ipc/queries/gte";
import { useMailCount } from "@/ipc/queries/mail";
import { useStartReindex } from "@/ipc/queries/reindex";
import {
  MIN_SEARCH_LEN,
  useDeleteSavedSearch,
  useSaveSearch,
  useSavedSearches,
  useSearch,
} from "@/ipc/queries/search";
import { cn } from "@/lib/cn";

import {
  ACCT_BREAKDOWN,
  IMPACT_ICON,
  QUICK_CHIPS,
  SUGGEST_PEOPLE,
  SUGGEST_TOPICS,
  SUGGEST_TRY,
  TODAY_DECISIONS,
  type AcctKey,
  type ImpactKind,
} from "./data";

const IMPACTS: ImpactKind[] = ["risk", "reply", "identity", "rule", "context"];
function asImpact(s: string): ImpactKind {
  return (IMPACTS as string[]).includes(s) ? (s as ImpactKind) : "context";
}
function fmtDecisionTime(sec: number): string {
  const d = new Date(sec * 1000);
  const now = new Date();
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return hm;
  const yest = new Date(now);
  yest.setDate(now.getDate() - 1);
  if (d.toDateString() === yest.toDateString()) return `Yesterday ${hm}`;
  return d.toLocaleDateString();
}

function fmtMB(bytes: number): string {
  return `${Math.round(bytes / 1_048_576).toLocaleString()} MB`;
}

type AcctFilter = "all" | AcctKey;
type DateFilter = "all" | "30" | "90" | "1y";
type SortBy = "date" | "rel" | "acct";
type SearchMode = "semantic" | "keyword" | "structured";
type EngineStatus = "active" | "indexing" | "paused";

const ESCAPE_RE = /[.*+?^${}()|[\]\\]/g;

// Account-filter chips map to the account's design-system colour token.
const ACCT_COLOR: Record<AcctKey, string> = { legal: "terra", work: "slate", person: "sage" };

function highlightHtml(text: string, q: string): string {
  if (!q) return text;
  const terms = q.toLowerCase().split(/\s+/).filter(Boolean);
  let out = text;
  terms.forEach((t) => {
    const re = new RegExp("(" + t.replace(ESCAPE_RE, "\\$&") + ")", "gi");
    out = out.replace(re, '<span class="hl">$1</span>');
  });
  return out;
}

// IA §4.4 mode detection: quoted phrase / single short token → keyword; field ops or
// natural language → hybrid (internal id "structured"); otherwise semantic.
function detectSearchMode(q: string): SearchMode {
  if (!q) return "semantic";
  if (/"[^"]+"/.test(q)) return "keyword";
  if (/\b(from:|to:|before:|after:|subject:)/i.test(q)) return "structured";
  const t = q.trim();
  if (!/\s/.test(t) && t.length < 4) return "keyword";
  return "structured";
}

const TOPIC_CHIPS: { id: string; n: number }[] = [
  { id: "Contract", n: 47 },
  { id: "Payment", n: 61 },
  { id: "Vendor", n: 105 },
  { id: "NDA", n: 18 },
  { id: "Schedule", n: 34 },
  { id: "Compliance", n: 23 },
];

export default function Repository() {
  const { t } = useTranslation("repository");

  // Real backend surfaces (mock-backed in the browser via src/ipc/client.ts).
  const accounts = useAccounts();
  const totalMail = useMailCount({});
  const qc = useQueryClient();
  const statsQ = useGteStats();
  const s = statsQ.data;
  const topicData = useTopicBreakdown().data ?? [];
  const knowledgeQ = useKnowledgeEntries();
  const knowledgeData: KnowledgeEntry[] = knowledgeQ.data ?? [];
  const decisionsQuery = useAiDecisions(EMPTY_AUDIT_FILTERS);
  const savedQuery = useSavedSearches();
  const saveSearch = useSaveSearch();
  const deleteSavedSearch = useDeleteSavedSearch();
  const reindexMutation = useStartReindex();

  // ── search state ──
  const [inputValue, setInputValue] = useState("");
  const [searchQ, setSearchQ] = useState("");
  const [searchMode, setSearchMode] = useState<SearchMode>("semantic");
  const [manualMode, setManualMode] = useState(false);
  const [suggestOpen, setSuggestOpen] = useState(false);
  const [recent, setRecent] = useState<string[]>(["vendor payment history", "boss@corp.com contract"]);
  const searchTimer = useRef<ReturnType<typeof setTimeout>>();
  const heroRef = useRef<HTMLDivElement>(null);

  // Live knowledge search — fires the real keyword/semantic command once the query
  // reaches the backend minimum length; below that we filter the curated browse set.
  const liveActive = searchQ.length >= MIN_SEARCH_LEN;
  const liveSearch = useSearch(searchMode === "keyword" ? "keyword" : "semantic", {
    query: searchQ,
    enabled: liveActive,
  });
  const liveResults: SearchResult[] = useMemo(() => liveSearch.data?.items ?? [], [liveSearch.data]);

  // Today's AI decisions from the audit log; fall back to the curated seed while the
  // query is loading or empty so the panel is never blank.
  const todayDecisions = useMemo(() => {
    const rows = decisionsQuery.data;
    if (!rows || rows.length === 0) return TODAY_DECISIONS;
    return rows.slice(0, 6).map((r: AiDecisionRow) => ({
      time: fmtDecisionTime(r.createdAt),
      email: r.mailSubject ?? "—",
      action: r.actionDescription,
      impact: asImpact(r.impact),
      basis: r.knowledgeSummary ?? (r.knowledgeRefs.length ? r.knowledgeRefs.join(", ") : "—"),
      result: r.resultDescription,
    }));
  }, [decisionsQuery.data]);

  const savedList: SavedSearch[] = savedQuery.data ?? [];
  const acctById = useMemo(
    () => new Map((accounts.data ?? []).map((a) => [a.id, a] as const)),
    [accounts.data],
  );

  // ── filter / sort state ──
  const [acctFilter, setAcctFilter] = useState<AcctFilter>("all");
  const [topicFilter, setTopicFilter] = useState<Set<string>>(new Set());
  const [dateFilter, setDateFilter] = useState<DateFilter>("all");
  const [sortBy, setSortBy] = useState<SortBy>("date");
  const [expandedId, setExpandedId] = useState<string | null>(null);

  // ── advanced drawer state ──
  const [c3Open, setC3Open] = useState(false);

  // ── reindex state ──
  const [indexing, setIndexing] = useState(false);
  const [paused, setPaused] = useState(false);
  const [pct, setPct] = useState(0);
  const [etaSecs, setEtaSecs] = useState<number | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [engineStatus, setEngineStatus] = useState<EngineStatus>("active");
  const [vectors, setVectors] = useState(87230);
  const [queue, setQueue] = useState<number>(12);
  const [indexVersion, setIndexVersion] = useState("v47");
  const [synced, setSynced] = useState(false);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;

  // ── derived: filter → sort ──
  const items = useMemo(() => {
    const now = Date.now();
    const cutoff =
      dateFilter === "30"
        ? now - 30 * 864e5
        : dateFilter === "90"
          ? now - 90 * 864e5
          : dateFilter === "1y"
            ? now - 365 * 864e5
            : null;

    const wantColor = acctFilter === "all" ? null : ACCT_COLOR[acctFilter];
    const ql = searchQ.toLowerCase();
    const filtered = knowledgeData.filter((item) => {
      if (wantColor && item.acctColor !== wantColor) return false;
      if (topicFilter.size > 0 && !item.tags.some((tag) => topicFilter.has(tag))) return false;
      if (cutoff != null && item.dateSent * 1000 < cutoff) return false;
      if (ql)
        return (item.subject + " " + item.excerpt + " " + item.tags.join(" ")).toLowerCase().includes(ql);
      return true;
    });

    const sorted = [...filtered];
    if (sortBy === "acct") {
      const order: Record<string, number> = { terra: 0, slate: 1, sage: 2 };
      sorted.sort((a, b) => (order[a.acctColor] ?? 9) - (order[b.acctColor] ?? 9));
    } else {
      sorted.sort((a, b) => b.dateSent - a.dateSent);
    }
    return sorted;
  }, [knowledgeData, acctFilter, topicFilter, dateFilter, sortBy, searchQ]);

  const resultText = useMemo(() => {
    if (liveActive) {
      if (liveSearch.isLoading) return t("searching");
      if (liveResults.length === 0) return t("result_none");
      const acctN = new Set(liveResults.map((r) => r.accountId)).size;
      return t("result_query", { count: liveResults.length, q: searchQ, accounts: acctN });
    }
    if (items.length === 0) return t("result_none");
    if (searchQ) {
      const acctN = new Set(items.map((x) => x.accountId)).size;
      return t("result_query", { count: items.length, q: searchQ, accounts: acctN });
    }
    return t("result_showing", { count: items.length });
  }, [items, searchQ, t, liveActive, liveSearch.isLoading, liveResults]);

  const hasFilters = acctFilter !== "all" || topicFilter.size > 0 || dateFilter !== "all";

  // ── search handlers ──
  const commitSearch = useCallback(
    (val: string) => {
      const q = val.trim();
      setSearchQ(q);
      setSortBy(q ? "rel" : "date");
      if (q && !manualMode) setSearchMode(detectSearchMode(q));
      if (!q) setManualMode(false);
      if (q.length > 2) setRecent((r) => [q, ...r.filter((x) => x !== q)].slice(0, 5));
    },
    [manualMode],
  );

  function onInput(val: string) {
    setInputValue(val);
    if (val.length > 0) setSuggestOpen(false);
    if (searchTimer.current) clearTimeout(searchTimer.current);
    searchTimer.current = setTimeout(() => commitSearch(val), 220);
  }

  function applySearch(q: string) {
    setInputValue(q);
    setSuggestOpen(false);
    if (searchTimer.current) clearTimeout(searchTimer.current);
    commitSearch(q);
  }

  function clearSearch() {
    setInputValue("");
    setSearchQ("");
    setManualMode(false);
    setSuggestOpen(false);
    setSortBy("date");
  }

  function toggleSearchMode() {
    if (!searchQ) return;
    setManualMode(true);
    setSearchMode((m) => (m === "semantic" ? "keyword" : m === "keyword" ? "structured" : "semantic"));
  }

  const flatSuggestions = useMemo(
    () => [
      ...recent,
      ...SUGGEST_PEOPLE.map((s) => s.query),
      ...SUGGEST_TOPICS.map((s) => s.query),
      ...SUGGEST_TRY.map((s) => s.query),
    ],
    [recent],
  );
  const [activeSuggest, setActiveSuggest] = useState(-1);

  function onSearchKeydown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (!suggestOpen) {
      if (e.key === "Enter") applySearch(inputValue);
      else if (e.key === "Escape") clearSearch();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveSuggest((i) => Math.min(i + 1, flatSuggestions.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveSuggest((i) => Math.max(i - 1, -1));
    } else if (e.key === "Enter") {
      if (activeSuggest >= 0 && flatSuggestions[activeSuggest]) applySearch(flatSuggestions[activeSuggest]);
      else applySearch(inputValue);
    } else if (e.key === "Escape") {
      clearSearch();
    }
  }

  // Close suggestions on outside click (prototype: document click listener).
  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      if (heroRef.current && !heroRef.current.contains(e.target as Node)) setSuggestOpen(false);
    }
    document.addEventListener("click", onDocClick);
    return () => document.removeEventListener("click", onDocClick);
  }, []);

  // ── reindex animation ──
  function startReindex() {
    if (indexing) return;
    setConfirmOpen(true);
  }

  function runReindex() {
    setConfirmOpen(false);
    // Fire the real reindex command; the bar below gives immediate visual feedback
    // (live gte:progress events drive it under a real backend).
    reindexMutation.mutate(null);
    setIndexing(true);
    setPaused(false);
    setPct(0);
    setEngineStatus("indexing");
  }

  useEffect(() => {
    if (!indexing) return;
    const start = Date.now();
    const id = setInterval(() => {
      if (pausedRef.current) return;
      setPct((prev) => {
        const next = Math.min(100, prev + Math.random() * 8 + 2);
        const elapsed = (Date.now() - start) / 1000;
        setEtaSecs(next > 0 && next < 100 ? Math.round((elapsed / next) * (100 - next)) : null);
        setQueue(Math.max(0, Math.round((100 - next) * 915)));
        if (next >= 100) {
          clearInterval(id);
          setTimeout(() => {
            const added = Math.floor(Math.random() * 30 + 8);
            setIndexing(false);
            setEngineStatus("active");
            setVectors(91450 + added);
            setQueue(0);
            setIndexVersion("v" + (Math.floor(Date.now() / 1000) % 10000));
            setSynced(true);
            void qc.invalidateQueries({ queryKey: ["gte_stats"] });
            void qc.invalidateQueries({ queryKey: ["topic_breakdown"] });
            showToast(t("toast_reindex_done", { count: added }));
          }, 500);
        }
        return next;
      });
    }, 250);
    return () => clearInterval(id);
  }, [indexing, t, qc]);

  // Keep engine-panel base values in sync with the backend stats when idle; the
  // reindex animation overrides them while running.
  useEffect(() => {
    if (s && !indexing) {
      setVectors(s.vectorCount);
      setQueue(s.queuePending);
      setIndexVersion(s.indexVersion);
    }
  }, [s, indexing]);

  function toggleIndexPause() {
    setPaused((p) => {
      const np = !p;
      setEngineStatus(np ? "paused" : "indexing");
      return np;
    });
  }

  // ── advanced drawer ──
  function c3Save() {
    const q = inputValue.trim() || t("c3_unnamed");
    saveSearch.mutate({
      name: q,
      query: inputValue.trim() || q,
      mode: searchMode === "keyword" ? "keyword" : "semantic",
      accountId: null,
    });
    showToast(t("c3_saved_done", { q }));
  }
  function c3Apply() {
    showToast(t("c3_applied"));
    setC3Open(false);
  }
  function c3Clear() {
    showToast(t("c3_cleared"));
  }

  // ── filter helpers ──
  function toggleTopic(tag: string) {
    setTopicFilter((prev) => {
      const next = new Set(prev);
      if (next.has(tag)) next.delete(tag);
      else next.add(tag);
      return next;
    });
  }
  function filterByTag(tag: string) {
    setTopicFilter((prev) => (prev.has(tag) ? prev : new Set(prev).add(tag)));
  }
  function clearAllFilters() {
    setAcctFilter("all");
    setTopicFilter(new Set());
    setDateFilter("all");
  }

  const IMPACT_LABEL: Record<ImpactKind, string> = {
    risk: t("impact_risk"),
    reply: t("impact_reply"),
    identity: t("impact_identity"),
    rule: t("impact_rule"),
    context: t("impact_context"),
  };
  const modeLabel: Record<SearchMode, string> = {
    semantic: t("mode_semantic"),
    keyword: t("mode_keyword"),
    structured: t("mode_hybrid"),
  };
  const modeHint: Record<SearchMode, string> = {
    semantic: t("hint_semantic"),
    keyword: t("hint_keyword"),
    structured: t("hint_hybrid"),
  };
  const modeSwitch: Record<SearchMode, string> = {
    semantic: t("sw_keyword"),
    keyword: t("sw_hybrid"),
    structured: t("sw_semantic"),
  };

  let flatIndex = -1;
  const suggestRow = (s: { icon: string; text: string; sub?: string; query: string }) => {
    flatIndex += 1;
    const idx = flatIndex;
    return (
      <div
        key={s.query + s.text}
        className={cn("suggest-item", idx === activeSuggest && "active")}
        onClick={() => applySearch(s.query)}
      >
        <span className="suggest-item-icon">{s.icon}</span>
        {s.text}
        {s.sub && <span className="suggest-item-sub">{s.sub}</span>}
      </div>
    );
  };

  return (
    <div className="page active" style={{ height: "100%" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {/* Compact title bar */}
        <div className="repo-header">
          <div className="repo-hdr-row">
            <div>
              <div className="repo-sub">{t("sub")}</div>
              <div className="repo-title">{t("title")}</div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <div className="gte-badge">
                <span className="gte-dot"></span>
                {t("badge")}
              </div>
              <button className={cn("reindex-btn", indexing && "running")} onClick={startReindex}>
                {indexing ? t("reindexing") : t("reindex")}
              </button>
            </div>
          </div>
          {searchQ && (
            <div className="search-mode-bar" style={{ display: "flex" }}>
              <span className={cn("search-mode-tag", searchMode)}>{modeLabel[searchMode]}</span>
              <span className="search-mode-text">{modeHint[searchMode]}</span>
              <span className="search-mode-switch" onClick={toggleSearchMode}>
                {modeSwitch[searchMode]}
              </span>
            </div>
          )}
        </div>

        {/* Search hero */}
        <div className="search-hero-section">
          <div className="search-hero-wrap" ref={heroRef}>
            <div className="search-hero-inner">
              <div className="search-hero-icon-wrap">
                <svg width="18" height="18" viewBox="0 0 16 16" fill="none">
                  <circle cx="6.5" cy="6.5" r="5" stroke="currentColor" strokeWidth="1.5" />
                  <path d="M10.5 10.5l4 4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
                </svg>
              </div>
              <input
                className="search-hero-input"
                placeholder={t("search_ph")}
                value={inputValue}
                onChange={(e) => onInput(e.target.value)}
                onFocus={() => {
                  if (!searchQ) setSuggestOpen(true);
                }}
                onKeyDown={onSearchKeydown}
              />
              {inputValue.length > 0 && (
                <span className="search-hero-clear vis" onClick={clearSearch}>
                  ×
                </span>
              )}
              <button
                className={cn("search-hero-btn", "c3-adv-btn", c3Open && "on")}
                onClick={() => setC3Open((o) => !o)}
              >
                {t("advanced")}
              </button>
              <button className="search-hero-btn" onClick={() => applySearch(inputValue)}>
                {t("search_btn")}
              </button>
            </div>

            {suggestOpen && (
              <div className="search-suggest open">
                {recent.length > 0 && (
                  <div className="suggest-section">
                    <div className="suggest-section-lbl">{t("suggest_recent")}</div>
                    {recent.map((q) =>
                      suggestRow({ icon: "🕐", text: q, query: q }),
                    )}
                  </div>
                )}
                <div className="suggest-section">
                  <div className="suggest-section-lbl">{t("suggest_people")}</div>
                  {SUGGEST_PEOPLE.map(suggestRow)}
                </div>
                <div className="suggest-section">
                  <div className="suggest-section-lbl">{t("suggest_topics")}</div>
                  {SUGGEST_TOPICS.map(suggestRow)}
                </div>
                <div className="suggest-section">
                  <div className="suggest-section-lbl">{t("suggest_try")}</div>
                  {SUGGEST_TRY.map(suggestRow)}
                </div>
              </div>
            )}
          </div>

          {!searchQ && (
            <div className="search-quick">
              <span className="search-quick-lbl">{t("quick_try")}</span>
              {QUICK_CHIPS.map((c) => (
                <span
                  key={c.query}
                  className="search-quick-chip"
                  title={c.title}
                  onClick={() => applySearch(c.query)}
                >
                  {c.label}
                </span>
              ))}
            </div>
          )}

          {c3Open && (
            <AdvancedDrawer
              t={t}
              saved={savedList}
              onSave={c3Save}
              onApply={c3Apply}
              onClear={c3Clear}
              onApplySaved={(s) => applySearch(s.query)}
              onDelSaved={(s) => deleteSavedSearch.mutate(s.id)}
            />
          )}
        </div>

        {/* Body */}
        <div className="repo-body">
          {/* ENTRIES PANE */}
          <div className="entries-pane">
            {!searchQ && (
              <div className="repo-today" style={{ margin: "16px 24px 0" }}>
                <div className="repo-today-hdr">
                  <span className="repo-today-title">{t("today_title")}</span>
                  <span className="repo-today-sub">{t("today_sub")}</span>
                </div>
                <div>
                  {todayDecisions.map((d, i) => (
                    <div className="today-item" key={i}>
                      <div className={cn("today-impact-icon", d.impact)}>{IMPACT_ICON[d.impact] || "✦"}</div>
                      <div className="today-main">
                        <div className="today-top">
                          <span className="today-time">{d.time}</span>
                          <span className="today-email">{d.email}</span>
                          <span className={cn("today-action-lbl", d.impact)}>{d.action}</span>
                        </div>
                        <div className="today-chain">
                          <div className="today-chain-block">
                            <span className="today-chain-lbl">{t("today_knowledge")}</span>
                            <span className="today-chain-val">{d.basis}</span>
                          </div>
                          <div className="today-chain-arrow">→</div>
                          <div className="today-chain-block">
                            <span className="today-chain-lbl">{t("today_result")}</span>
                            <span className="today-chain-result-val">{d.result}</span>
                          </div>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Filter strip */}
            <div className="filter-strip" style={{ marginTop: 12 }}>
              <span className="filter-lbl">{t("filter_account")}</span>
              {(["all", "legal", "work", "person"] as AcctFilter[]).map((a) => (
                <span
                  key={a}
                  className={cn("acct-tab", `t-${a}`, acctFilter === a && "on")}
                  onClick={() => setAcctFilter(a)}
                >
                  {t(a === "all" ? "tab_all" : a === "person" ? "tab_person" : `tab_${a}`)}
                </span>
              ))}
              <span className="div-dot">·</span>
              <span className="filter-lbl">{t("filter_topic")}</span>
              {TOPIC_CHIPS.map((c) => (
                <span
                  key={c.id}
                  className={cn("topic-chip", topicFilter.has(c.id) && "on")}
                  title={c.id === "NDA" ? "Non-Disclosure Agreement" : undefined}
                  onClick={() => toggleTopic(c.id)}
                >
                  {c.id} <span className="ct">{c.n}</span>
                </span>
              ))}
              <span className="div-dot">·</span>
              <span className="filter-lbl">{t("filter_period")}</span>
              {(["all", "30", "90", "1y"] as DateFilter[]).map((d) => (
                <span
                  key={d}
                  className={cn("date-chip", dateFilter === d && "on")}
                  onClick={() => setDateFilter(d)}
                >
                  {t(`date_${d}`)}
                </span>
              ))}
              {hasFilters && (
                <span className="clear-filters-btn" style={{ display: "inline-flex" }} onClick={clearAllFilters}>
                  {t("clear_filters")}
                </span>
              )}
            </div>

            {/* Result bar */}
            <div className="result-bar">
              <span className="result-count">{resultText}</span>
              <div className="sort-row">
                <span className="sort-lbl">{t("sort_lbl")}</span>
                {(["date", "rel", "acct"] as SortBy[]).map((s) => (
                  <span key={s} className={cn("sort-opt", sortBy === s && "on")} onClick={() => setSortBy(s)}>
                    {t(`sort_${s}`)}
                  </span>
                ))}
              </div>
            </div>

            {/* Entries list */}
            <div className="entries-list">
              {liveActive ? (
                liveSearch.isLoading ? (
                  <div className="empty-state">
                    <div className="empty-title">{t("searching")}</div>
                  </div>
                ) : liveResults.length === 0 ? (
                  <EmptyState t={t} />
                ) : (
                  liveResults.map((r, i) => (
                    <SearchResultCard
                      key={r.mailId}
                      result={r}
                      index={i}
                      searchQ={searchQ}
                      searchMode={searchMode}
                      account={acctById.get(r.accountId)}
                      t={t}
                    />
                  ))
                )
              ) : items.length === 0 ? (
                <EmptyState t={t} />
              ) : (
                items.map((entry, i) => (
                  <KnowledgeCard
                    key={entry.id}
                    entry={entry}
                    index={i}
                    expanded={expandedId === entry.id}
                    impactLabel={IMPACT_LABEL[asImpact(entry.impact)]}
                    onToggle={() => setExpandedId((id) => (id === entry.id ? null : entry.id))}
                    onTag={filterByTag}
                    t={t}
                  />
                ))
              )}
            </div>
          </div>

          {/* SIDEBAR */}
          <div className="repo-sidebar">
            <div className="stat-strip" style={{ marginBottom: 22 }}>
              <div className="stat-cell">
                <div className="stat-n">{vectors.toLocaleString()}</div>
                <div className="stat-l">{t("stat_vectors")}</div>
              </div>
              <div className="stat-cell">
                <div className="stat-n" style={{ color: "var(--green)" }}>
                  {s?.usedToday ?? 0}
                </div>
                <div className="stat-l">{t("stat_used_today")}</div>
              </div>
              <div className="stat-cell">
                <div className="stat-n" style={{ color: "var(--terra)" }}>
                  {s?.risksCaught ?? 0}
                </div>
                <div className="stat-l">{t("stat_risks")}</div>
              </div>
              <div className="stat-cell">
                <div className="stat-n">{(s?.emailCount ?? 0).toLocaleString()}</div>
                <div className="stat-l">{t("stat_emails")}</div>
              </div>
              <div className="stat-cell stat-wide">
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", gap: 8 }}>
                  <div>
                    <div className="stat-n">{(s?.coveragePct ?? 0).toFixed(1)}%</div>
                    <div className="stat-l">{t("stat_coverage")}</div>
                  </div>
                  <div className="stat-sub">
                    <a onClick={() => showToast(t("toast_unindexed"))}>
                      {t("stat_unindexed", { n: (s?.unindexedCount ?? 0).toLocaleString() })}
                    </a>
                  </div>
                </div>
                <div className="coverage-bar-track">
                  <div className="coverage-bar-fill" style={{ width: `${(s?.coveragePct ?? 0).toFixed(1)}%` }}></div>
                </div>
              </div>
            </div>

            {/* GTE Engine */}
            <div className="sb-sec">
              <div className="sb-sec-title">{t("engine_title")}</div>
              <div className="gte-panel">
                <EngineRow k={t("engine_status")}>
                  <span
                    className={cn("gte-val", engineStatus === "active" ? "ok" : engineStatus === "paused" ? "dim" : "warn")}
                  >
                    {engineStatus === "active"
                      ? t("engine_status_active")
                      : engineStatus === "paused"
                        ? t("engine_status_paused")
                        : t("engine_status_indexing")}
                  </span>
                </EngineRow>
                <EngineRow k={t("engine_model")}>
                  <span className="gte-val">{s?.model ?? "bge-m3"}</span>
                </EngineRow>
                <EngineRow k={t("engine_dimensions")}>
                  <span className="gte-val">{s?.dimensions ?? 1024}</span>
                </EngineRow>
                <EngineRow k={t("engine_index")}>
                  <span className="gte-val">{indexVersion}</span>
                </EngineRow>
                <EngineRow k={t("engine_last_sync")}>
                  <span className="gte-val dim">{synced ? t("engine_sync_now") : t("engine_sync_2min")}</span>
                </EngineRow>
                <EngineRow k={t("engine_storage")}>
                  <span className="gte-val">{fmtMB(s?.storageBytes ?? 0)}</span>
                </EngineRow>
                <EngineRow k={t("engine_queue")}>
                  <span className="gte-val warn">
                    {indexing ? queue.toLocaleString() : t("engine_queue_pending", { count: queue })}
                  </span>
                </EngineRow>
                <EngineRow k={t("engine_coverage")}>
                  <span className="gte-val">{(s?.coveragePct ?? 0).toFixed(1)}%</span>
                </EngineRow>
                <EngineRow k={t("engine_spam")}>
                  <span className="gte-val dim">{(s?.spamExcluded ?? 0).toLocaleString()}</span>
                </EngineRow>

                {indexing && (
                  <div className="progress-wrap" style={{ display: "block" }}>
                    <div className="progress-header">
                      <span className="progress-lbl">{t("progress_indexing", { pct: Math.round(pct) })}</span>
                      <span className="progress-eta">
                        {etaSecs != null ? t("progress_eta", { secs: etaSecs }) : t("progress_calc")}
                      </span>
                    </div>
                    <div className="progress-track">
                      <div className="progress-fill" style={{ width: `${pct}%` }}></div>
                    </div>
                    <div className="progress-note">{t("progress_note")}</div>
                    <div className="progress-footer">
                      <span className="progress-pause-btn" onClick={toggleIndexPause}>
                        {paused ? t("progress_resume") : t("progress_pause")}
                      </span>
                    </div>
                  </div>
                )}

                <div className="gte-actions-row">
                  <span className="gte-action-link" onClick={() => showToast(t("toast_clean"))}>
                    {t("clean_vectors")}
                  </span>
                  <span className="gte-action-link" onClick={() => showToast(t("toast_export"))}>
                    {t("export_stats")}
                  </span>
                </div>
              </div>
            </div>

            {/* Account breakdown */}
            <div className="sb-sec">
              <div className="sb-sec-title">{t("sb_accounts")}</div>
              <div>
                {(accounts.data ?? []).length > 0
                  ? (accounts.data ?? []).map((a) => (
                      <RepoAcctRow key={a.id} account={a} totalRef={totalMail.data ?? 1} />
                    ))
                  : ACCT_BREAKDOWN.map((d) => (
                      <SeedAcctRow key={d.lbl} lbl={d.lbl} addr={d.addr} color={d.color} tc={d.tc} n={d.n} pct={Math.round((d.n / d.max) * 100)} />
                    ))}
              </div>
            </div>

            {/* Topic chart */}
            <div className="sb-sec">
              <div className="sb-sec-title">{t("sb_topics")}</div>
              <div>
                {(() => {
                  const max = Math.max(1, ...topicData.map((d) => d.count));
                  return topicData.map((d) => (
                    <div className="topic-bar-row" key={d.label} onClick={() => filterByTag(d.label)}>
                      <div className="topic-bar-lbl">{d.label}</div>
                      <div className="topic-bar-track">
                        <div
                          className="topic-bar-fill"
                          style={{ width: `${Math.round((d.count / max) * 100)}%`, background: `var(--${d.color})` }}
                        ></div>
                      </div>
                      <div className="topic-bar-n">{d.count}</div>
                    </div>
                  ));
                })()}
              </div>
            </div>
          </div>
        </div>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        title={t("reindex_title")}
        body={t("reindex_body")}
        confirmLabel={t("reindex_confirm")}
        onConfirm={runReindex}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  );
}

function EngineRow({ k, children }: { k: string; children: React.ReactNode }) {
  return (
    <div className="gte-row">
      <span className="gte-key">{k}</span>
      {children}
    </div>
  );
}

type TFn = (key: string, opts?: Record<string, unknown>) => string;

function KnowledgeCard({
  entry,
  index,
  expanded,
  impactLabel,
  onToggle,
  onTag,
  t,
}: {
  entry: KnowledgeEntry;
  index: number;
  expanded: boolean;
  impactLabel: string;
  onToggle: () => void;
  onTag: (tag: string) => void;
  t: TFn;
}) {
  const impact = asImpact(entry.impact);
  const acctTc = entry.acctColor === "sage" || entry.acctColor === "amber" ? "var(--p10)" : undefined;
  const usedText = entry.usedCount > 0 ? t("used_by_ai", { count: entry.usedCount }) : t("not_used");

  return (
    <div
      className={cn("k-card", expanded && "expanded")}
      style={{ animationDelay: `${index * 30}ms` }}
      onClick={(e) => {
        if ((e.target as HTMLElement).classList.contains("k-tag")) return;
        onToggle();
      }}
    >
      <div className="k-rel-bar" style={{ background: "transparent" }}></div>
      <div className="k-top">
        <div
          className="k-acct-chip"
          style={{ background: `var(--${entry.acctColor})`, ...(acctTc ? { color: acctTc } : {}) }}
        >
          {entry.acctBadge}
        </div>
        <div className="k-main">
          <div className="k-title" style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
            <span>{entry.subject}</span>
            <span className={cn("k-impact-badge", impact)}>{impactLabel}</span>
            <span className={cn("k-used-badge", entry.usedCount > 0 && "active")}>{usedText}</span>
          </div>
          <div className="k-excerpt">{entry.excerpt}</div>
          {entry.lastUsedFor && (
            <div className={cn("k-causal", impact)}>
              <span className="k-causal-lbl">{t("last_used")}</span>
              <span className="k-causal-val">{entry.lastUsedFor}</span>
              <span className="k-causal-time">
                {entry.lastUsedTime != null ? fmtCardDate(entry.lastUsedTime) : ""}
              </span>
            </div>
          )}
        </div>
        <div className="k-right">
          <span className="k-date">{fmtCardDate(entry.dateSent)}</span>
        </div>
      </div>
      <div className="k-tags">
        {entry.tags.map((tag) => (
          <span
            className="k-tag"
            key={tag}
            onClick={(e) => {
              e.stopPropagation();
              onTag(tag);
            }}
          >
            {tag}
          </span>
        ))}
      </div>
      <div className="k-expand">
        <div className="k-meta-grid">
          <div className="k-meta-cell">
            <div className="k-meta-lbl">{t("meta_source")}</div>
            <div className="k-meta-val">{entry.source}</div>
          </div>
          <div className="k-meta-cell">
            <div className="k-meta-lbl">{t("meta_thread")}</div>
            <div className="k-meta-val">{entry.thread}</div>
          </div>
          <div className="k-meta-cell">
            <div className="k-meta-lbl">{t("meta_indexed")}</div>
            <div className="k-meta-val">{entry.indexedAt != null ? fmtCardDate(entry.indexedAt) : "—"}</div>
          </div>
        </div>
        <div className="k-expand-body" dangerouslySetInnerHTML={{ __html: entry.body }} />
        <div className="k-expand-actions">
          <button
            className="k-action-btn"
            onClick={(e) => {
              e.stopPropagation();
              showToast(t("toast_open_email"));
            }}
          >
            {t("open_email")}
          </button>
          <button
            className="k-action-btn"
            onClick={(e) => {
              e.stopPropagation();
              showToast(t("toast_copy"));
            }}
          >
            {t("copy_summary")}
          </button>
        </div>
      </div>
    </div>
  );
}

function AdvancedDrawer({
  t,
  saved,
  onSave,
  onApply,
  onClear,
  onApplySaved,
  onDelSaved,
}: {
  t: TFn;
  saved: SavedSearch[];
  onSave: () => void;
  onApply: () => void;
  onClear: () => void;
  onApplySaved: (s: SavedSearch) => void;
  onDelSaved: (s: SavedSearch) => void;
}) {
  // The drawer chips are visual refinements (the live filter strip drives results,
  // matching the prototype). Local on/off state keeps them interactive.
  const [groups, setGroups] = useState<Record<string, Set<string>>>({
    mode: new Set(["Keyword"]),
    accounts: new Set(["Legal", "Work", "Personal", "Backup"]),
    time: new Set(["Any time"]),
    filters: new Set(),
    folder: new Set(["All"]),
    sender: new Set(),
    tags: new Set(),
  });
  const radio = (g: string) => g === "mode" || g === "time" || g === "folder";
  const toggle = (g: string, v: string) =>
    setGroups((prev) => {
      const next = new Set(radio(g) ? [] : prev[g]);
      if (radio(g)) next.add(v);
      else if (next.has(v)) next.delete(v);
      else next.add(v);
      return { ...prev, [g]: next };
    });

  const grp = (g: string, label: string, opts: { v: string; title?: string }[]) => (
    <div className="c3-grp">
      <div className="c3-grp-lbl">{label}</div>
      <div className="c3-chips">
        {opts.map((o) => (
          <span
            key={o.v}
            className={cn("c3-chip", groups[g]?.has(o.v) && "on")}
            title={o.title}
            onClick={() => toggle(g, o.v)}
          >
            {o.v}
          </span>
        ))}
      </div>
    </div>
  );

  return (
    <div className="c3-drawer" style={{ display: "block" }}>
      <div className="c3-grid">
        {grp("mode", t("c3_mode"), [{ v: t("mode_keyword") }, { v: t("mode_semantic") }, { v: t("mode_hybrid") }])}
        {grp("accounts", t("c3_accounts"), [
          { v: "Legal" },
          { v: "Work" },
          { v: "Personal" },
          { v: "Backup" },
        ])}
        {grp("time", t("c3_time"), [{ v: t("t_any") }, { v: t("t_7d") }, { v: t("t_month") }, { v: t("t_year") }])}
        {grp("filters", t("c3_filters"), [
          { v: t("f_unread") },
          { v: t("f_flagged") },
          { v: t("f_attach") },
          { v: t("f_starred") },
        ])}
        {grp("folder", t("c3_folder"), [
          { v: t("fold_all") },
          { v: t("fold_inbox") },
          { v: t("fold_archive") },
          { v: t("fold_sent") },
        ])}
        {grp("sender", t("c3_sender"), [{ v: "Boss Senior" }, { v: "AP Finance" }, { v: "Project Manager" }])}
        {grp("tags", t("c3_tags"), [{ v: "Contract" }, { v: "Invoice" }, { v: "Client A" }])}
      </div>
      <div className="c3-drawer-foot">
        <div className="c3-saved">
          {saved.length === 0 ? (
            <span className="c3-saved-empty">{t("c3_no_saved")}</span>
          ) : (
            saved.map((s) => (
              <span className="c3-saved-chip" key={s.id} onClick={() => onApplySaved(s)}>
                ⭐ {s.name}{" "}
                <span
                  className="c3-saved-x"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelSaved(s);
                  }}
                >
                  ×
                </span>
              </span>
            ))
          )}
        </div>
        <div className="c3-drawer-actions">
          <button className="set-btn" onClick={onSave}>
            {t("c3_save")}
          </button>
          <button className="set-btn" onClick={onClear}>
            {t("c3_clear")}
          </button>
          <button className="set-btn primary" onClick={onApply}>
            {t("c3_apply")}
          </button>
        </div>
      </div>
    </div>
  );
}

function EmptyState({ t }: { t: TFn }) {
  return (
    <div className="empty-state">
      <svg width="44" height="44" viewBox="0 0 48 48" fill="none">
        <circle cx="20" cy="20" r="13" stroke="#AEA89F" strokeWidth="1.5" />
        <path d="M30 30l10 10" stroke="#AEA89F" strokeWidth="1.5" strokeLinecap="round" />
        <path d="M15 20h10M20 15v10" stroke="#AEA89F" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
      <div className="empty-title">{t("empty_title")}</div>
      <div className="empty-sub">{t("empty_sub")}</div>
    </div>
  );
}

function fmtCardDate(sec: number): string {
  return new Date(sec * 1000).toLocaleDateString();
}

/** A live search hit rendered with the knowledge-card shell (no expand body — the
 *  full record opens in the reading view). Driven by keyword/semantic_search. */
function SearchResultCard({
  result,
  index,
  searchQ,
  searchMode,
  account,
  t,
}: {
  result: SearchResult;
  index: number;
  searchQ: string;
  searchMode: SearchMode;
  account: Account | undefined;
  t: TFn;
}) {
  const score = result.score;
  const relColor = score >= 0.7 ? "var(--green)" : score >= 0.4 ? "var(--amber)" : "var(--p6)";
  const scoreClass = score >= 0.7 ? "score-high" : score >= 0.4 ? "score-mid" : "score-low";
  const scoreLabel = score >= 0.7 ? t("rel_high") : score >= 0.4 ? t("rel_mid") : t("rel_low");
  const basisMode =
    searchMode === "keyword" ? t("mode_keyword") : searchMode === "semantic" ? t("mode_semantic") : t("mode_hybrid");
  const chip = account?.badgeLabel ?? "·";
  const chipBg = account ? `var(--${account.colorToken})` : "var(--p9)";
  const chipTc = account && (account.colorToken === "sage" || account.colorToken === "amber") ? "var(--p10)" : undefined;
  const excerptHtml = result.highlights.length
    ? result.highlights.join(" … ").replaceAll("<mark>", '<span class="hl">').replaceAll("</mark>", "</span>")
    : highlightHtml(result.snippet, searchQ);

  return (
    <div className="k-card" style={{ animationDelay: `${index * 30}ms` }}>
      <div className="k-rel-bar" style={{ background: relColor }}></div>
      <div className="k-top">
        <div className="k-acct-chip" style={{ background: chipBg, ...(chipTc ? { color: chipTc } : {}) }}>
          {chip}
        </div>
        <div className="k-main">
          <div className="k-title">
            <span dangerouslySetInnerHTML={{ __html: highlightHtml(result.subject, searchQ) }} />
          </div>
          <div className="k-excerpt" dangerouslySetInnerHTML={{ __html: excerptHtml }} />
        </div>
        <div className="k-right">
          <span className="k-basis">
            {t("basis")} {basisMode}
          </span>
          <span className={cn("k-score", scoreClass)}>{scoreLabel}</span>
          <span className="k-date">{fmtCardDate(result.dateSent)}</span>
        </div>
      </div>
    </div>
  );
}

/** Sidebar account-breakdown row backed by a live per-account mail count. */
function RepoAcctRow({ account, totalRef }: { account: Account; totalRef: number }) {
  const count = useMailCount({ accountId: account.id });
  const n = count.data ?? 0;
  const pct = totalRef > 0 ? Math.max(6, Math.min(100, Math.round((n / totalRef) * 100))) : 6;
  const tc = account.colorToken === "sage" || account.colorToken === "amber" ? "var(--p10)" : undefined;
  return (
    <SeedAcctRow
      lbl={account.displayName}
      addr={account.email}
      color={`var(--${account.colorToken})`}
      tc={tc}
      n={n}
      pct={pct}
      chip={account.badgeLabel}
    />
  );
}

function SeedAcctRow({
  lbl,
  addr,
  color,
  tc,
  n,
  pct,
  chip,
}: {
  lbl: string;
  addr: string;
  color: string;
  tc?: string;
  n: number;
  pct: number;
  chip?: string;
}) {
  return (
    <div>
      <div className="acct-row">
        <div className="acct-mini" style={{ background: color, ...(tc ? { color: tc } : {}) }}>
          {chip ?? lbl[0]}
        </div>
        <div className="acct-info">
          <div className="acct-rname">{lbl}</div>
          <div className="acct-raddr">{addr}</div>
        </div>
      </div>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "0 0 10px 0",
          borderBottom: "1px solid var(--p5)",
          marginBottom: 2,
        }}
      >
        <div style={{ flex: 1, height: 5, background: "var(--p5)", borderRadius: 3, overflow: "hidden" }}>
          <div style={{ width: `${pct}%`, height: "100%", background: color, borderRadius: 3, transition: "width .7s var(--ease)" }}></div>
        </div>
        <span style={{ fontFamily: "var(--fm)", fontSize: 10, color: "var(--p8)", flexShrink: 0, minWidth: 44, textAlign: "right" }}>
          {n.toLocaleString()}
        </span>
      </div>
    </div>
  );
}
