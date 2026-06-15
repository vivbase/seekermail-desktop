// TanStack Query hooks for the E7 audit-log surface (T089) plus the shared
// trust-downgrade mutation (T086 §3 / T089 §3). Components consume these,
// never `ipc()` directly (07 §6). The T088 backend commands are mocked in
// `client.ts` until the Rust Wave-3 surface registers.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type {
  AiDecisionRow,
  DecisionSummary,
  ExportAiDecisionsParams,
  IpcError,
} from "@shared/bindings";

import { showToast } from "@/components/ui/Toast";
import { EMPTY_AI_SETTINGS_PATCH } from "../aiSettings";
import { ipc } from "../client";
import { accountKeys } from "./accounts";

export const auditKeys = {
  root: ["audit"] as const,
  list: (filters: AuditListFilters) => ["audit", "list", filters] as const,
  summary: (accountId: string | null, sinceUnix: number, untilUnix: number) =>
    ["audit", "summary", accountId ?? "all", sinceUnix, untilUnix] as const,
  approvedCount: (accountId: string) => ["audit", "approved_count", accountId] as const,
};

/** Hard cap for the v0.7 list — beyond this the UI shows a narrow-filters hint. */
export const AUDIT_LIST_LIMIT = 200;

export interface AuditListFilters {
  accountIds: string[];
  eventTypes: string[];
  /** Unix seconds; null = unbounded. */
  sinceUnix: number | null;
  untilUnix: number | null;
}

export const EMPTY_AUDIT_FILTERS: AuditListFilters = {
  accountIds: [],
  eventTypes: [],
  sinceUnix: null,
  untilUnix: null,
};

/**
 * Filtered decision list (T089 §3). `list_ai_decisions` takes a single
 * optional accountId, so a multi-account selection fetches unscoped and
 * filters client-side (the list is capped at 200 rows anyway).
 */
export function useAiDecisions(filters: AuditListFilters) {
  return useQuery({
    queryKey: auditKeys.list(filters),
    queryFn: async (): Promise<AiDecisionRow[]> => {
      const rows = await ipc("list_ai_decisions", {
        params: {
          accountId: filters.accountIds.length === 1 ? (filters.accountIds[0] ?? null) : null,
          sinceUnix: filters.sinceUnix,
          untilUnix: filters.untilUnix,
          decisionTypes: filters.eventTypes.length > 0 ? filters.eventTypes : null,
          impact: null,
          limit: AUDIT_LIST_LIMIT,
          offset: null,
        },
      });
      if (filters.accountIds.length <= 1) return rows;
      const wanted = new Set(filters.accountIds);
      return rows.filter((r) => wanted.has(r.accountId));
    },
    staleTime: 30_000,
  });
}

/** Summary-bar aggregates (T089 §3); cached for a minute per F_E7 §4.6. */
export function useAiDecisionsSummary(
  accountId: string | null,
  sinceUnix: number,
  untilUnix: number,
) {
  return useQuery({
    queryKey: auditKeys.summary(accountId, sinceUnix, untilUnix),
    queryFn: (): Promise<DecisionSummary> =>
      ipc("get_ai_decisions_summary", { accountId, sinceUnix, untilUnix }),
    staleTime: 60_000,
  });
}

/** CSV/JSON export; resolves to the written file path (T088). */
export function useExportAiDecisions() {
  return useMutation<string, IpcError, ExportAiDecisionsParams>({
    mutationFn: (params) => ipc("export_ai_decisions", { params }),
  });
}

/**
 * Lifetime count of approved (sent) drafts for one account — the E3 unlock
 * gate reads `approvedDraftCount >= 50` (T086, F_E3 §4.1). Derived from
 * `list_ai_decisions` filtered to `draft_sent`; no extra IPC command.
 */
export function useApprovedDraftCount(accountId: string) {
  return useQuery({
    queryKey: auditKeys.approvedCount(accountId),
    queryFn: async (): Promise<number> => {
      const rows = await ipc("list_ai_decisions", {
        params: {
          accountId,
          sinceUnix: 0,
          untilUnix: null,
          decisionTypes: ["draft_sent"],
          impact: null,
          limit: 1000,
          offset: null,
        },
      });
      return rows.filter((r) => r.decisionType === "draft_sent").length;
    },
    enabled: !!accountId,
    staleTime: 60_000,
  });
}

// ── Mis-send feedback → trust downgrade (T086 §3 / T089 §3) ───────────────────

/** Mis-send reports older than this no longer count toward the downgrade. */
const MIS_SEND_WINDOW_SECS = 7 * 86_400;

/** Reports within the window required to demote the account to Semi-Auto. */
export const MIS_SEND_DEMOTE_THRESHOLD = 3;

/** `app_settings` key holding the JSON timestamp array for one account. */
export function misSendSettingKey(accountId: string): string {
  return `ai.mis_send_${accountId}`;
}

export interface ReportMisSendVars {
  accountId: string;
}

/**
 * Record one "this should not have been sent" report. The timestamps live in
 * `app_settings` as a raw JSON array (no DB column, AI_MODES_DESIGN §9.4);
 * crossing the 7-day threshold demotes the account to Semi-Auto via
 * `update_account_ai_settings` and announces it with a toast. Resolves with
 * `true` when a demotion happened.
 */
export function useReportMisSend() {
  const qc = useQueryClient();
  const { t } = useTranslation(["aiDrafts", "audit"]);

  return useMutation<boolean, IpcError, ReportMisSendVars>({
    mutationFn: async ({ accountId }) => {
      const key = misSendSettingKey(accountId);
      const raw = await ipc("get_setting", { key });
      let timestamps: number[] = [];
      if (raw !== null) {
        try {
          const parsed: unknown = JSON.parse(raw);
          if (Array.isArray(parsed)) {
            timestamps = parsed.filter((v): v is number => typeof v === "number");
          }
        } catch {
          // Malformed value → start a fresh window rather than fail the report.
        }
      }
      const now = Math.floor(Date.now() / 1000);
      const recent = timestamps.filter((ts) => now - ts < MIS_SEND_WINDOW_SECS);
      recent.push(now);
      await ipc("set_setting", { key, value: JSON.stringify(recent) });

      if (recent.length >= MIS_SEND_DEMOTE_THRESHOLD) {
        await ipc("update_account_ai_settings", {
          account_id: accountId,
          params: { ...EMPTY_AI_SETTINGS_PATCH, authLevel: 2 },
        });
        return true;
      }
      return false;
    },
    onSuccess: (demoted, { accountId }) => {
      void qc.invalidateQueries({ queryKey: auditKeys.root });
      if (demoted) {
        void qc.invalidateQueries({ queryKey: accountKeys.aiSettings(accountId) });
        void qc.invalidateQueries({ queryKey: accountKeys.all });
        showToast(t("aiDrafts:trust_demoted_notice"));
      } else {
        showToast(t("audit:audit_mis_send_feedback_toast"));
      }
    },
  });
}

/** T089 card name for the same hook (T086 calls it useReportMisSend). */
export const useMisSend = useReportMisSend;
