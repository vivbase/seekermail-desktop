// Search page (T034, promoted from the global overlay to a first-class route).
// Route: /search — reachable from the left rail like every other primary page and
// from the global Cmd/Ctrl+K shortcut (AppShell navigates here). It owns the same
// query-routing, dual-mode, account-filter and saved/recent-search behaviour the
// old SearchPanel overlay had, laid out as a full page so it follows the app's
// "every destination is a page" navigation logic.
//
// Query routing (Auto mode, when the user hasn't pinned a mode):
//   - Contains field prefixes (from:/subject:/to:/has:/in:) → keyword only
//   - Contains a quoted "…" phrase → keyword only
//   - Otherwise → both keyword + semantic in parallel, merged and deduped by mailId
//
// Keyboard: ArrowUp/Down moves the result selection · Enter opens the highlighted
// result. No Esc-to-close — this is a page, not a modal.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";

import type { AttachmentHit, IpcError, SearchResult } from "@shared/bindings";
import {
  useKeywordSearch,
  useSearchHistory,
  useSearchWithAttachments,
  useSemanticSearch,
  MIN_SEARCH_LEN,
} from "@/ipc/queries/search";
import { useUi } from "@/stores/ui";
import { cn } from "@/lib/cn";
import { AccountFilterBar } from "@/components/search/AccountFilterBar";
import { SearchResultList } from "@/components/search/SearchResultList";
import { SaveSearchDialog } from "@/components/search/SaveSearchDialog";
import { SavedSearchList } from "@/components/search/SavedSearchList";

// ── Debounce hook ─────────────────────────────────────────────────────────────

function useDebounce<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(id);
  }, [value, delayMs]);
  return debounced;
}

// ── Query routing logic ───────────────────────────────────────────────────────

const FIELD_PREFIX_RE = /\b(from|subject|to|has|in):/i;
const QUOTED_RE = /"[^"]+"/;

type ResolvedMode = "keyword" | "semantic" | "both";

function resolveMode(query: string, uiMode: "keyword" | "semantic"): ResolvedMode {
  // Explicit user choice is always honoured.
  if (uiMode === "keyword") return "keyword";
  if (uiMode === "semantic") return "semantic";
  // Auto: field prefixes or quoted phrases → keyword.
  if (FIELD_PREFIX_RE.test(query) || QUOTED_RE.test(query)) return "keyword";
  return "both";
}

// ── Magnifying-glass icon ─────────────────────────────────────────────────────

function SearchIcon({ className }: { className?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 20 20"
      fill="currentColor"
      className={className}
      aria-hidden
    >
      <path
        fillRule="evenodd"
        d="M9 3.5a5.5 5.5 0 100 11 5.5 5.5 0 000-11zM2 9a7 7 0 1112.452 4.391l3.328 3.329a.75.75 0 11-1.06 1.06l-3.329-3.328A7 7 0 012 9z"
        clipRule="evenodd"
      />
    </svg>
  );
}

// ── Search page ───────────────────────────────────────────────────────────────

export default function SearchRoute() {
  const { t } = useTranslation("search");
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();

  const searchMode = useUi((s) => s.searchMode);
  const setSearchMode = useUi((s) => s.setSearchMode);
  const searchAccountFilter = useUi((s) => s.searchAccountFilter);

  // Seed the input from the `?q=` param (Cmd+K and saved-search deep links).
  const [rawQuery, setRawQuery] = useState(() => searchParams.get("q") ?? "");
  const debouncedQuery = useDebounce(rawQuery, 250);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [saveDialogOpen, setSaveDialogOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const trimmed = debouncedQuery.trim();
  const queryActive = trimmed.length >= MIN_SEARCH_LEN;

  // Resolve the effective mode for the parallel dual-query logic.
  const resolved = queryActive ? resolveMode(trimmed, searchMode) : "keyword";

  const kwEnabled = queryActive && (resolved === "keyword" || resolved === "both");
  const keyword = useKeywordSearch({
    query: trimmed,
    accountFilter: searchAccountFilter,
    enabled: kwEnabled,
  });

  const semEnabled = queryActive && (resolved === "semantic" || resolved === "both");
  const semantic = useSemanticSearch({
    query: trimmed,
    accountFilter: searchAccountFilter,
    enabled: semEnabled,
  });

  // Attachment-origin hits (T110): we use only its attachmentHits — mail hits keep
  // flowing through the keyword/semantic hooks so Auto-mode dual querying is preserved.
  const attachmentSearch = useSearchWithAttachments({
    query: trimmed,
    mode: resolved === "both" ? "auto" : resolved,
    accountFilter: searchAccountFilter,
    enabled: queryActive,
  });
  const attachmentHits = attachmentSearch.data?.attachmentHits ?? [];

  const history = useSearchHistory();

  // Autofocus the input when the page mounts (so Cmd+K → /search lands ready to type).
  useEffect(() => {
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  // Keep the `?q=` param in sync without spamming history entries.
  useEffect(() => {
    const current = searchParams.get("q") ?? "";
    if (rawQuery === current) return;
    const next = new URLSearchParams(searchParams);
    if (rawQuery) next.set("q", rawQuery);
    else next.delete("q");
    setSearchParams(next, { replace: true });
  }, [rawQuery, searchParams, setSearchParams]);

  // Merged + deduped results for "both" mode.
  const mergedItems = useMemo<SearchResult[]>(() => {
    if (!queryActive) return [];

    let base: SearchResult[];
    if (resolved === "keyword") {
      base = keyword.data?.items ?? [];
    } else if (resolved === "semantic") {
      base = semantic.data?.items ?? [];
    } else {
      // Both: merge by mailId, prefer the higher score.
      const byId = new Map<string, SearchResult>();
      for (const r of [...(keyword.data?.items ?? []), ...(semantic.data?.items ?? [])]) {
        const existing = byId.get(r.mailId);
        if (!existing || r.score > existing.score) byId.set(r.mailId, r);
      }
      base = [...byId.values()].sort((a, b) => b.score - a.score);
    }

    // Cross-account client-side scope (T113). Empty filter = all accounts.
    if (searchAccountFilter.length > 0) {
      const selected = new Set(searchAccountFilter);
      base = base.filter((r) => selected.has(r.accountId));
    }
    return base;
  }, [resolved, queryActive, keyword.data, semantic.data, searchAccountFilter]);

  const isLoading = (kwEnabled && keyword.isLoading) || (semEnabled && semantic.isLoading);

  // GTE_INDEX_CORRUPT inline hint.
  const gteError = useMemo<IpcError | null>(() => {
    if (semantic.error) {
      const e = semantic.error as unknown as IpcError;
      if (e.code === "GTE_INDEX_CORRUPT") return e;
    }
    return null;
  }, [semantic.error]);

  const showSemanticFallback =
    resolved === "both" &&
    !semantic.isLoading &&
    (semantic.data?.items.length ?? 0) === 0 &&
    (keyword.data?.items.length ?? 0) > 0;

  const isEmpty = !isLoading && queryActive && mergedItems.length === 0 && !gteError;

  // Reset the selection when the result set changes.
  useEffect(() => {
    setSelectedIndex(0);
  }, [trimmed, resolved, searchAccountFilter]);

  const openResult = useCallback(
    (result: SearchResult) => {
      void navigate(`/mail/${result.mailId}`);
    },
    [navigate],
  );

  const openAttachment = useCallback(
    (hit: AttachmentHit) => {
      void navigate(`/mail/${hit.mailId}?attachmentId=${encodeURIComponent(hit.attachmentId)}`);
    },
    [navigate],
  );

  function handleInputKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (!queryActive) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, mergedItems.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const result = mergedItems[selectedIndex];
      if (result) openResult(result);
    }
  }

  function runQuery(q: string) {
    setRawQuery(q);
    setSelectedIndex(0);
    inputRef.current?.focus();
  }

  const showLanding = rawQuery.trim().length === 0;
  const hasResults = mergedItems.length > 0;
  const recent = history.data ?? [];

  return (
    <div className="flex h-full w-full flex-col px-7 py-6">
      {/* Page header (matches the app's italic-serif page titles) */}
      <header className="mb-4 shrink-0">
        <h1 className="font-display text-2xl italic text-p10">{t("panel_title")}</h1>
        <p className="mt-1 font-body text-sm text-p7">{t("page_sub")}</p>
      </header>

      {/* Search input row + mode toggle (token-styled surface card) */}
      <div className="shrink-0 rounded-card border border-divider bg-surface shadow-card">
        <div className="flex items-center gap-2 px-4 py-3">
          <SearchIcon className="h-4 w-4 shrink-0 text-p7" />
          <input
            ref={inputRef}
            type="search"
            value={rawQuery}
            onChange={(e) => {
              setRawQuery(e.target.value);
              setSelectedIndex(0);
            }}
            onKeyDown={handleInputKeyDown}
            placeholder={t("placeholder")}
            autoComplete="off"
            spellCheck={false}
            className="min-w-0 flex-1 bg-transparent font-body text-base text-p10 placeholder:text-p7 focus:outline-none"
          />

          {/* Mode toggle */}
          <div
            role="group"
            aria-label={t("mode_keyword") + " / " + t("mode_semantic")}
            className="flex shrink-0 items-center rounded-chip border border-divider bg-parchment"
          >
            {(["keyword", "semantic"] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                onClick={() => setSearchMode(mode)}
                aria-pressed={searchMode === mode}
                className={cn(
                  "rounded-chip px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider transition-colors",
                  searchMode === mode ? "bg-p9 text-white" : "text-p8 hover:bg-p4",
                )}
              >
                {mode === "keyword" ? t("mode_keyword") : t("mode_semantic")}
              </button>
            ))}
          </div>
        </div>

        {/* Account filter chip row (T113) — self-hides with ≤1 account */}
        <AccountFilterBar />

        {/* Too-short hint */}
        {rawQuery.trim().length > 0 && !queryActive && (
          <p className="border-t border-divider px-4 py-2.5 font-ui text-xs text-p7">
            {t("too_short")}
          </p>
        )}

        {/* GTE unavailable hint */}
        {gteError && (
          <div className="bg-amber/10 border-t border-divider px-4 py-2">
            <p className="font-ui text-xs text-amber">{t("gte_unavailable")}</p>
          </div>
        )}
      </div>

      {/* Body: landing (saved + recent) when idle, results when querying */}
      <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
        {showLanding ? (
          <div className="flex flex-col gap-6">
            {/* Saved searches */}
            <section>
              <p className="section-label mb-2">{t("saved_title")}</p>
              <SavedSearchList onRun={(q) => runQuery(q)} />
            </section>

            {/* Recent searches */}
            <section>
              <p className="section-label mb-2">{t("history_title")}</p>
              {recent.length === 0 ? (
                <p className="font-body text-sm italic text-p7">{t("recent_empty")}</p>
              ) : (
                <div className="flex flex-col gap-0.5">
                  {recent.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => {
                        if (item.mode === "keyword" || item.mode === "semantic") {
                          setSearchMode(item.mode);
                        }
                        runQuery(item.query);
                      }}
                      className="flex items-center gap-2 rounded-chip px-2 py-1.5 text-start transition-colors hover:bg-p4"
                    >
                      <SearchIcon className="h-3.5 w-3.5 shrink-0 text-p7" />
                      <span className="min-w-0 truncate font-body text-sm text-p9">
                        {item.query}
                      </span>
                      <span className="shrink-0 font-mono text-[10px] uppercase text-p7">
                        {item.mode}
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </section>
          </div>
        ) : (
          <div className="rounded-card border border-divider bg-surface">
            {/* Save-this-search CTA */}
            {hasResults && (
              <div className="flex justify-end border-b border-divider px-4 py-1.5">
                <button
                  type="button"
                  onClick={() => setSaveDialogOpen(true)}
                  className="font-ui text-xs text-p7 hover:text-p9 hover:underline"
                >
                  {t("save_action")} →
                </button>
              </div>
            )}

            <SearchResultList
              data={{
                items: mergedItems,
                total:
                  resolved === "both"
                    ? mergedItems.length
                    : resolved === "keyword"
                      ? (keyword.data?.total ?? mergedItems.length)
                      : (semantic.data?.total ?? mergedItems.length),
                offset: 0,
              }}
              isLoading={isLoading}
              isEmpty={isEmpty}
              showSemanticFallback={showSemanticFallback}
              selectedIndex={selectedIndex}
              onSelect={(result, idx) => {
                setSelectedIndex(idx);
                openResult(result);
              }}
              attachmentHits={attachmentHits}
              onSelectAttachment={openAttachment}
              listMaxHeight="calc(100vh - 320px)"
            />
          </div>
        )}
      </div>

      {/* Save-search dialog */}
      <SaveSearchDialog
        open={saveDialogOpen}
        query={trimmed}
        onClose={() => setSaveDialogOpen(false)}
      />
    </div>
  );
}
