// TanStack Query hooks for the mail surfaces (T029 reading view + T036 list hooks).
// Components consume these, never `ipc()` directly (07 §6).
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  ImageAllowScope,
  ListMailsParams,
  ListThreadsParams,
  MailDetail,
  PageResult,
  Thread,
} from "@shared/bindings";

import { ipc } from "../client";

/** Default page size for list queries (02 §Pagination). */
export const PAGE_SIZE = 50;

export const mailKeys = {
  trackerInfo: (mailId: string) => ["tracker_info", mailId] as const,
  threads: (params: Omit<ListThreadsParams, "limit" | "offset">) =>
    ["threads", params.accountId ?? "all", params] as const,
  mails: (params: Omit<ListMailsParams, "limit" | "offset">) =>
    ["mails", params.accountId ?? "all", params] as const,
  detail: (mailId: string) => ["mail", mailId] as const,
};

// ── Reading view (T029) ──────────────────────────────────────────────────────

/** Tracker status + sender image-allow state for one mail. */
export function useTrackerInfo(mailId: string) {
  return useQuery({
    queryKey: mailKeys.trackerInfo(mailId),
    queryFn: () => ipc("get_tracker_info", { mail_id: mailId }),
    enabled: !!mailId,
    staleTime: 60_000,
  });
}

/** Allow remote images for a message or always for its sender. */
export function useAllowRemoteImages() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { mailId: string; scope: ImageAllowScope }) =>
      ipc("allow_remote_images", { mail_id: vars.mailId, scope: vars.scope }),
    onSuccess: (_d, vars) =>
      void qc.invalidateQueries({ queryKey: mailKeys.trackerInfo(vars.mailId) }),
  });
}

/**
 * Resolve a mail's inline (cid:) images to bytes for in-body rendering. Inline
 * parts carry no privacy cost (they ship inside the message), so this is enabled
 * automatically — but only when the body actually references `cid:`, so plain
 * mail never triggers a backend fetch.
 */
export function useInlineImages(mailId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["inline_images", mailId] as const,
    queryFn: () => ipc("get_inline_images", { mail_id: mailId }),
    enabled: enabled && !!mailId,
    staleTime: 5 * 60_000,
  });
}

/**
 * Fetch one remote image through the backend (no cookies / Referer / UA) for a
 * `data:` URI swap, so the webview never connects to the origin. Used by the
 * remote-image bar's load action and the allow-listed-sender auto-reveal.
 */
export function useFetchRemoteImage() {
  return useMutation({
    mutationFn: (url: string) => ipc("fetch_remote_image", { url }),
  });
}

// ── L0 thread stream (T036/T037) ─────────────────────────────────────────────

type ThreadFilter = Omit<ListThreadsParams, "limit" | "offset">;

/** Infinite, paginated thread list — backs the L0 card stream. */
export function useThreadsInfinite(filter: ThreadFilter) {
  return useInfiniteQuery({
    queryKey: mailKeys.threads(filter),
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      ipc("list_threads", { params: { ...filter, limit: PAGE_SIZE, offset: pageParam } }),
    getNextPageParam: (last: PageResult<Thread>) => {
      const next = last.offset + last.items.length;
      return next < last.total ? next : undefined;
    },
    staleTime: 15_000,
  });
}

/** Flatten an infinite thread query into a single array. */
export function flattenThreads(data: { pages: PageResult<Thread>[] } | undefined): Thread[] {
  return data ? data.pages.flatMap((p) => p.items) : [];
}

// ── Flat mail list (T036/T047/T048) ──────────────────────────────────────────

type MailFilter = Omit<ListMailsParams, "limit" | "offset">;

/** Infinite, paginated flat mail list — unread / processed / all-mail routes. */
export function useMailsInfinite(filter: MailFilter) {
  return useInfiniteQuery({
    queryKey: mailKeys.mails(filter),
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      ipc("list_mails", { params: { ...filter, limit: PAGE_SIZE, offset: pageParam } }),
    getNextPageParam: (last) => {
      const next = last.offset + last.items.length;
      return next < last.total ? next : undefined;
    },
    staleTime: 15_000,
  });
}

/** Count-only query for a mail filter — drives the Dashboard stat cards. Keyed
 * under ["mails", …] so the sync/new-mail event invalidation refreshes it. */
export function useMailCount(filter: MailFilter) {
  return useQuery({
    queryKey: ["mails", "count", filter] as const,
    queryFn: async () => {
      const page = await ipc("list_mails", { params: { ...filter, limit: 1, offset: 0 } });
      return page.total;
    },
    staleTime: 15_000,
  });
}

// ── L2 detail (T041) ─────────────────────────────────────────────────────────

/** Full mail detail for the reading view. */
export function useMailDetail(mailId: string | null | undefined) {
  return useQuery({
    queryKey: mailKeys.detail(mailId ?? ""),
    queryFn: () => ipc("get_mail", { mail_id: mailId as string }),
    enabled: !!mailId,
    staleTime: 30_000,
  });
}

// ── Mutations (T038/T041) ────────────────────────────────────────────────────

/** Mark a mail read/unread, patching the detail cache optimistically. */
export function useSetMailRead() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { mailId: string; isRead: boolean }) =>
      ipc("set_mail_read", { mail_id: vars.mailId, is_read: vars.isRead }),
    onMutate: async (vars) => {
      await qc.cancelQueries({ queryKey: mailKeys.detail(vars.mailId) });
      const prev = qc.getQueryData<MailDetail>(mailKeys.detail(vars.mailId));
      if (prev) qc.setQueryData(mailKeys.detail(vars.mailId), { ...prev, isRead: vars.isRead });
      return { prev };
    },
    onError: (_e, vars, ctx) => {
      if (ctx?.prev) qc.setQueryData(mailKeys.detail(vars.mailId), ctx.prev);
    },
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ["threads"] });
      void qc.invalidateQueries({ queryKey: ["mails"] });
    },
  });
}

/** Star/unstar a mail. */
export function useSetMailStarred() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { mailId: string; isStarred: boolean }) =>
      ipc("set_mail_starred", { mail_id: vars.mailId, is_starred: vars.isStarred }),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ["threads"] });
      void qc.invalidateQueries({ queryKey: ["mails"] });
    },
  });
}

/** Archive a mail (removes it from the active streams). */
export function useArchiveMail() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (mailId: string) => ipc("archive_mail", { mail_id: mailId }),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ["threads"] });
      void qc.invalidateQueries({ queryKey: ["mails"] });
    },
  });
}

/** Soft-delete a mail. */
export function useDeleteMail() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (mailId: string) => ipc("delete_mail", { mail_id: mailId }),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ["threads"] });
      void qc.invalidateQueries({ queryKey: ["mails"] });
    },
  });
}

/** Restore a trashed mail back to the Inbox (analysis/44 §5). */
export function useRestoreMail() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (mailId: string) => ipc("restore_mail", { mail_id: mailId }),
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ["threads"] });
      void qc.invalidateQueries({ queryKey: ["mails"] });
    },
  });
}
