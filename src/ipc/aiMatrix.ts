// Hand-written mirrors of the Rust F4 matrix DTOs (src-tauri/src/ai/matrix.rs,
// T065; wire shapes are camelCase per the generated-bindings conventions).
// These types exist so T066's matrix UI is fully typed today; once
// `pnpm gen:types` emits them into `@shared/bindings`, delete this file and
// import from there instead.

import type { AiProvider } from "./aiSettings";

/**
 * Routable AI capability (ai/types.rs). The wire/JSON spelling is the
 * PascalCase variant name — this is what the persisted matrix carries.
 */
export type Capability = "DraftReply" | "RiskReason" | "Summarize" | "StyleProfile";

/** Row order the matrix UI presents capabilities in (mirrors `ALL_CAPABILITIES`). */
export const ALL_CAPABILITIES: readonly Capability[] = [
  "DraftReply",
  "RiskReason",
  "Summarize",
  "StyleProfile",
] as const;

/** Hard cap on the backup-chain length per cell (F_F4 §6). */
export const MAX_BACKUPS = 2;

/** One provider + model choice inside a cell (F_F4 §4.2). */
export type ProviderAssignment = {
  provider: AiProvider;
  /** Model name at that provider. Empty string = "the provider's default". */
  model: string;
  /** Custom endpoint override; `null` = the provider's standard endpoint. */
  baseUrl: string | null;
};

/**
 * The assignment for one `(capability, account)` cell: a required primary plus
 * an ordered fallback chain of at most {@link MAX_BACKUPS} links (F_F4 §4.2).
 */
export type MatrixCell = {
  primary: ProviderAssignment;
  backups: ProviderAssignment[];
};

/** One matrix row binding for an account: capability → cell. */
export type MatrixEntry = {
  capability: Capability;
  cell: MatrixCell;
};

/**
 * The full per-account matrix (F_F4 §4.4). An empty `entries` list is valid —
 * every capability then falls back to the account's base provider columns.
 */
export type CapabilityMatrix = {
  entries: MatrixEntry[];
};

/**
 * One non-blocking save-time hint (F_F4 §4.5). `code` is a stable tag the UI
 * maps to localized copy; `message` is the English fallback.
 */
export type MatrixWarning = {
  capability: Capability;
  code: string;
  message: string;
};

/** One item of a `batch_update_provider_matrix` call (F_F4 §4.3). */
export type BatchMatrixUpdate = {
  accountId: string;
  capability: Capability;
  cell: MatrixCell;
};

/** Local (on-device) provider check — drives the cell's Local/Cloud badge. */
export function isLocalProvider(provider: AiProvider): boolean {
  return provider === "ollama" || provider === "local_onnx";
}

/** The cell routing `capability`, when the matrix has one (mirrors `CapabilityMatrix::cell`). */
export function matrixCellOf(
  matrix: CapabilityMatrix | undefined,
  capability: Capability,
): MatrixCell | null {
  return matrix?.entries.find((e) => e.capability === capability)?.cell ?? null;
}

/** Insert or replace the cell for `capability`, preserving entry order (immutable). */
export function withMatrixCell(
  matrix: CapabilityMatrix,
  capability: Capability,
  cell: MatrixCell,
): CapabilityMatrix {
  const index = matrix.entries.findIndex((e) => e.capability === capability);
  const entries = matrix.entries.slice();
  if (index >= 0) entries[index] = { capability, cell };
  else entries.push({ capability, cell });
  return { entries };
}

/** Remove the cell for `capability` — the backend then falls back to defaults. */
export function withoutMatrixCell(
  matrix: CapabilityMatrix,
  capability: Capability,
): CapabilityMatrix {
  return { entries: matrix.entries.filter((e) => e.capability !== capability) };
}
