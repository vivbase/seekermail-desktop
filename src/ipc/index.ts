// Public IPC surface. Components import hooks/helpers from here, never from
// `@tauri-apps/api` directly (07 §6).
export { ipc, isTauri, normalizeIpcError } from "./client";
export type { Commands, CommandName } from "./client";
export { EMPTY_AI_SETTINGS_PATCH } from "./aiSettings";
export type {
  AccountAiSettings,
  AiProvider,
  AuthLevel,
  UpdateAiSettingsParams,
} from "./aiSettings";
export {
  LEGAL_LEVEL_COLOR_VAR,
  LEGAL_LEVEL_WEIGHT,
  LEGAL_OVERALL_COLOR_VAR,
  riskEventLevelColorVar,
  T4_RISK_LEVEL,
} from "./legal";
export type {
  AnalyzeLegalRiskParams,
  LegalAnalysisResult,
  LegalKeyClauses,
  LegalOverallLevel,
  LegalRiskItem,
  LegalRiskLevel,
  LegalRiskType,
  ListRiskEventsParams,
  ResolveRiskParams,
  RiskEvent,
  RiskStatus,
} from "./legal";
export { ERROR_UX, uxForCode } from "./errors";
export type { ErrorBucket, Affordance, ErrorUx } from "./errors";
export { registerIpcEvents, registerAllEvents, useEvent } from "./events";
export type { IpcEventName } from "./events";
export * from "./queries/system";
export * from "./queries/accounts";
export * from "./queries/mail";
export * from "./queries/attachments";
export * from "./queries/settings";
export * from "./queries/export";
export * from "./queries/wipe";
export * from "./queries/reindex";
export * from "./queries/risk";
