// Hand-written mirrors of the Rust data-flow disclosure DTOs (T069,
// src-tauri/src/commands/data_flow.rs). These types exist so the data-flow
// panel is fully typed today; once `pnpm gen:types` emits them into
// `@shared/bindings`, delete this file and import from there instead (field
// shapes follow the generated-bindings conventions: camelCase, `| null`).

import type { AiProvider } from "./aiSettings";

/**
 * Where one account's AI requests terminate (dev/06 §8): a cloud endpoint
 * (mail content leaves the device), a localhost daemon, an in-process model,
 * or nowhere because AI is off.
 */
export type AiEndpointKind = "cloud" | "local" | "in_process" | "none";

/** One per-account AI routing row for the disclosure panel. */
export type AiRouteEntry = {
  accountId: string;
  accountEmail: string;
  colorToken: string;
  aiProvider: AiProvider;
  aiModel: string | null;
  endpointKind: AiEndpointKind;
  /** Full effective endpoint URL; `null` for in-process / disabled rows. */
  endpointUrl: string | null;
  /** Display authority only (`api.openai.com`, `localhost:11434`). */
  endpointHost: string | null;
  /** `true` when the provider never sends mail content off this device. */
  isLocal: boolean;
};

/**
 * One aggregated `ai_decisions` bucket for the 24h summary — identifiers,
 * counts, and token totals only; never prompt, completion, or mail content.
 */
export type AiActivityRow = {
  accountId: string;
  decisionType: string;
  aiModel: string | null;
  requestCount: number;
  inputTokens: number;
  outputTokens: number;
};

/** `get_data_flow_ai_routing` payload: routing rows + 24h activity summary. */
export type DataFlowAiRouting = {
  routes: AiRouteEntry[];
  activity: AiActivityRow[];
  /** Unix seconds of the summary window start (`now - 24h`). */
  sinceUnix: number;
};
