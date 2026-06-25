// TanStack Query hook for the compose "AI Draft" flow (analysis/57 §7). Wraps
// the `generate_compose_draft` IPC command (commands::ai), which builds a prompt
// from the user's intent and returns generated body text. Components consume
// this hook, never `ipc()` directly (07 §6).
import { useMutation } from "@tanstack/react-query";

import type { ComposeDraftResult, GenerateComposeDraftParams, IpcError } from "@shared/bindings";
import { ipc } from "../client";

/**
 * Generate an ephemeral compose body (a forwarding note, or a new-mail body)
 * from the user's intent. Nothing is persisted — the caller inserts the text
 * into the editor and the user sends manually.
 */
export function useGenerateComposeDraft() {
  return useMutation<ComposeDraftResult, IpcError, GenerateComposeDraftParams>({
    mutationFn: (params) => ipc("generate_compose_draft", { params }),
  });
}
