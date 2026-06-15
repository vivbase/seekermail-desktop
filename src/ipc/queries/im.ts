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
