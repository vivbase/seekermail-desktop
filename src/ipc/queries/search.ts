// TanStack Query hooks for search (C1 keyword / C2 semantic) + saved searches
// (T034/T035). Components consume these, never `ipc()` directly (07 §6).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  KeywordSearchParams,
  PageResult,
  SaveSearchParams,
  SearchResult,
  SearchWithAttachmentsParams,
  SearchWithAttachmentsResult,
  SemanticSearchParams,
} from "@shared/bindings";

import { ipc } from "../client";

export type SearchMode = "keyword" | "semantic";

/** Minimum query length before either search fires (matches the backend gate). */
export const MIN_SEARCH_LEN = 3;
const PAGE = 50;

export const searchKeys = {
  keyword: (q: string, accountId: string | null) => ["search", "keyword", accountId, q] as const,
  semantic: (q: string, accountId: string | null) => ["search", "semantic", accountId, q] as const,
  history: () => ["search_history"] as const,
  saved: () => ["saved_searches"] as const,
};

interface SearchArgs {
  query: string;
  accountId?: string | null;
  /**
   * Cross-account selection (T113). Empty/absent = all accounts. The hooks derive
   * the IPC shape: 1 id → `accountId` (single-account path); >1 → `accountFilter`
   * (semantic only — keyword has no subset param, so it queries all accounts and
   * the panel filters client-side by `accountId`).
   */
  accountFilter?: string[] | null;
  dateFrom?: number | null;
  dateTo?: number | null;
  enabled?: boolean;
}

/** The single account id implied by a filter, or null for all / multi. */
function singleAccount(accountId: string | null, accountFilter: string[] | null): string | null {
  if (accountId) return accountId;
  if (accountFilter && accountFilter.length === 1) return accountFilter[0] ?? null;
  return null;
}

/** Keyword (FTS5) search. Disabled until the query reaches [`MIN_SEARCH_LEN`]. */
export function useKeywordSearch({
  query,
  accountId = null,
  accountFilter = null,
  dateFrom = null,
  dateTo = null,
  enabled = true,
}: SearchArgs) {
  const effectiveAccountId = singleAccount(accountId, accountFilter);
  const params: KeywordSearchParams = {
    query,
    accountId: effectiveAccountId,
    dateFrom,
    dateTo,
    folder: null,
    limit: PAGE,
    offset: 0,
  };
  return useQuery<PageResult<SearchResult>>({
    queryKey: ["search", "keyword", effectiveAccountId, (accountFilter ?? []).join(","), query],
    queryFn: () => ipc("keyword_search", { params }),
    enabled: enabled && query.trim().length >= MIN_SEARCH_LEN,
    staleTime: 30_000,
  });
}

/** Semantic (ANN) search. Disabled until the query reaches [`MIN_SEARCH_LEN`]. */
export function useSemanticSearch({
  query,
  accountId = null,
  accountFilter = null,
  dateFrom = null,
  dateTo = null,
  enabled = true,
}: SearchArgs) {
  const effectiveAccountId = singleAccount(accountId, accountFilter);
  // Multi-account subset goes to the backend's account_filter (T112); a single
  // account or "all" uses the account_id path.
  const effectiveFilter =
    !effectiveAccountId && accountFilter && accountFilter.length > 1 ? accountFilter : null;
  const params: SemanticSearchParams = {
    query,
    accountId: effectiveAccountId,
    accountFilter: effectiveFilter,
    dateFrom,
    dateTo,
    minScore: null,
    limit: PAGE,
    offset: 0,
  };
  return useQuery<PageResult<SearchResult>>({
    queryKey: ["search", "semantic", effectiveAccountId, (accountFilter ?? []).join(","), query],
    queryFn: () => ipc("semantic_search", { params }),
    enabled: enabled && query.trim().length >= MIN_SEARCH_LEN,
    staleTime: 30_000,
  });
}

/** Route to keyword or semantic search by `mode`. */
export function useSearch(mode: SearchMode, args: SearchArgs) {
  const keyword = useKeywordSearch({
    ...args,
    enabled: args.enabled !== false && mode === "keyword",
  });
  const semantic = useSemanticSearch({
    ...args,
    enabled: args.enabled !== false && mode === "semantic",
  });
  return mode === "keyword" ? keyword : semantic;
}

interface AttachmentSearchArgs extends SearchArgs {
  /** Mail-hit search to combine with the attachment search (T110). */
  mode: "keyword" | "semantic" | "auto";
}

/**
 * Combined mail + attachment search (T110). Returns `{ mailHits, attachmentHits }`
 * in one round-trip. `staleTime: 0` so each query re-runs (matches the panel's
 * other search hooks).
 */
export function useSearchWithAttachments({
  query,
  mode,
  accountId = null,
  accountFilter = null,
  dateFrom = null,
  dateTo = null,
  enabled = true,
}: AttachmentSearchArgs) {
  const effectiveAccountId = singleAccount(accountId, accountFilter);
  const params: SearchWithAttachmentsParams = {
    query,
    mode,
    accountId: effectiveAccountId,
    dateFrom,
    dateTo,
    limit: 50,
  };
  return useQuery<SearchWithAttachmentsResult>({
    queryKey: [
      "search",
      "with_attachments",
      effectiveAccountId,
      (accountFilter ?? []).join(","),
      mode,
      query,
    ] as const,
    queryFn: () => ipc("search_with_attachments", { params }),
    enabled: enabled && query.trim().length >= MIN_SEARCH_LEN,
    staleTime: 0,
  });
}

interface UnifiedSearchArgs {
  query: string;
  accountFilter?: string[] | null;
  dateFrom?: number | null;
  dateTo?: number | null;
  enabled?: boolean;
}

/**
 * Unified cross-account search (T113): runs keyword + semantic + attachment
 * search in parallel, merges the mail hits (dedup by `mailId`, higher score
 * wins), and scopes to `accountFilter` (multi-account selections are filtered
 * client-side here since the keyword backend has no subset param).
 */
export function useUnifiedSearch({
  query,
  accountFilter = null,
  dateFrom = null,
  dateTo = null,
  enabled = true,
}: UnifiedSearchArgs) {
  const keyword = useKeywordSearch({ query, accountFilter, dateFrom, dateTo, enabled });
  const semantic = useSemanticSearch({ query, accountFilter, dateFrom, dateTo, enabled });
  const attachments = useSearchWithAttachments({
    query,
    mode: "auto",
    accountId: singleAccount(null, accountFilter),
    enabled,
  });

  const selected = new Set(accountFilter ?? []);
  const byId = new Map<string, SearchResult>();
  for (const r of [...(keyword.data?.items ?? []), ...(semantic.data?.items ?? [])]) {
    if (selected.size > 0 && !selected.has(r.accountId)) continue;
    const existing = byId.get(r.mailId);
    if (!existing || r.score > existing.score) byId.set(r.mailId, r);
  }
  const mailHits = [...byId.values()].sort((a, b) => b.score - a.score);

  return {
    mailHits,
    attachmentHits: attachments.data?.attachmentHits ?? [],
    isLoading: keyword.isLoading || semantic.isLoading || attachments.isLoading,
    isError: keyword.isError || semantic.isError || attachments.isError,
  };
}

/** Recent searches for the panel history dropdown. */
export function useSearchHistory() {
  return useQuery({
    queryKey: searchKeys.history(),
    queryFn: () => ipc("get_search_history", { limit: 20 }),
    staleTime: 30_000,
  });
}

/** All saved searches (sidebar). */
export function useSavedSearches() {
  return useQuery({
    queryKey: searchKeys.saved(),
    queryFn: () => ipc("list_saved_searches", undefined),
    staleTime: 60_000,
  });
}

/** Persist a saved search. */
export function useSaveSearch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: SaveSearchParams) => ipc("save_search", { params }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: searchKeys.saved() }),
  });
}

/** Delete a saved search. */
export function useDeleteSavedSearch() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => ipc("delete_saved_search", { id }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: searchKeys.saved() }),
  });
}
