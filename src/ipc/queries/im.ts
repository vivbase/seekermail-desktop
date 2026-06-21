// TanStack Query hooks for the Agent-IM (TEAM) channel (T093, consuming the T092
// commands). Components consume these, never `ipc()` directly (07 §6).
//
// The list polls every 5 s for now; once the Agent-IM event stream lands (T101)
// the `imMessages` cache is invalidated by `query:new` / `query:expired` and the
// interval can be dropped.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ipc } from "../client";
import { MAIN_CHANNEL, type ImMessage } from "../im";

/** Backend caps the channel at 5000 rows; the UI pulls the latest page. */
const PAGE_LIMIT = 200;

export const imKeys = {
  messages: ["imMessages"] as const,
  // Child of `messages` on purpose: invalidating `["imMessages"]` (the list)
  // prefix-matches this too, so the badge refreshes together with the timeline.
  unread: ["imMessages", "unread"] as const,
};

/** The shared channel timeline, oldest-first. */
export function useImMessages() {
  return useQuery({
    queryKey: imKeys.messages,
    queryFn: () =>
      ipc("list_im_messages", { sender_id: null, status: null, limit: PAGE_LIMIT, offset: 0 }),
    refetchInterval: 5000,
    staleTime: 2000,
  });
}

/** TEAM nav-badge count: unread agent messages + unresolved decision cards
 *  (T101). Polls so the badge stays live even when the operator is on another
 *  page; opening the channel (`useMarkTeamRead`) clears the unread half. */
export function useTeamUnreadCount() {
  return useQuery({
    queryKey: imKeys.unread,
    queryFn: () => ipc("count_team_unread"),
    refetchInterval: 8000,
    staleTime: 4000,
  });
}

/** Mark the whole shared channel read — call when the TEAM page opens. */
export function useMarkTeamRead() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => ipc("mark_im_channel_read", { channel_id: MAIN_CHANNEL }),
    // Refresh the timeline + badge (the unread key is a child of `messages`).
    onSuccess: () => void qc.invalidateQueries({ queryKey: imKeys.messages }),
  });
}

export interface PostImMessageVars {
  senderType: ImMessage["senderType"];
  senderId: string;
  messageType: ImMessage["messageType"];
  /** Already-serialized JSON content (use `textContent()` for plain text). */
  content: string;
  linkedEmailId?: string | null;
}

/** Post a message to the shared channel; the timeline refetches on success. */
export function usePostImMessage() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: PostImMessageVars) =>
      ipc("post_im_message", {
        channel_id: MAIN_CHANNEL,
        sender_type: vars.senderType,
        sender_id: vars.senderId,
        message_type: vars.messageType,
        content: vars.content,
        linked_email_id: vars.linkedEmailId ?? null,
      }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: imKeys.messages }),
  });
}

/** Mark one message read (idempotent backend). */
export function useMarkImMessageRead() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => ipc("mark_im_message_read", { id }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: imKeys.messages }),
  });
}
