// E7 detail drawer (T089 §3): side panel for one decision row. Privacy rule
// (dev/09 §5): `action_description` / `result_description` are NEVER shown —
// they are structurally absent from `AiDecisionRow`, and nothing here may
// reintroduce them. Sent events offer the "Report mis-send" feedback link
// that feeds the T086 trust-downgrade loop. No sheet primitive exists in the
// repo, so this follows the accessible-dialog pattern used by the dialogs.
import { useEffect, useRef, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import type { AiDecisionRow } from "@shared/bindings";

import { useAccounts } from "@/ipc/queries/accounts";
import { useReportMisSend } from "@/ipc/queries/audit";

import { eventColorVar, eventTypeLabel, supportsMisSendReport } from "./eventColor";

interface AuditDrawerProps {
  entry: AiDecisionRow | null;
  onClose: () => void;
}

export function AuditDrawer({ entry, onClose }: AuditDrawerProps) {
  const { t } = useTranslation("audit");
  const { data: accounts } = useAccounts();
  const reportMisSend = useReportMisSend();
  const panelRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);

  // Focus moves into the drawer on open and returns to the row on close.
  useEffect(() => {
    if (!entry) return;
    openerRef.current = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    return () => openerRef.current?.focus();
  }, [entry]);

  if (!entry) return null;

  const accountName =
    accounts?.find((a) => a.id === entry.accountId)?.displayName ?? entry.accountId;

  const handleKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key !== "Tab" || !panelRef.current) return;
    const focusables = panelRef.current.querySelectorAll<HTMLElement>(
      "button:not([disabled]), [href], input, select, textarea, [tabindex]:not([tabindex='-1'])",
    );
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  const fields: { label: string; value: string; mono?: boolean }[] = [
    { label: t("audit_drawer_account"), value: accountName },
    {
      label: t("audit_drawer_subject"),
      value: entry.mailSubject
        ? entry.mailSubject.length > 60
          ? `${entry.mailSubject.slice(0, 60)}…`
          : entry.mailSubject
        : "—",
    },
    {
      label: t("audit_drawer_time"),
      value: new Date(entry.createdAt * 1000).toISOString(),
      mono: true,
    },
    { label: t("audit_drawer_model"), value: entry.aiModel ?? "—" },
    { label: t("audit_drawer_impact"), value: entry.impact ?? "—" },
    {
      label: t("audit_drawer_tokens_in"),
      value: entry.inputTokens !== null ? entry.inputTokens.toLocaleString() : "—",
      mono: true,
    },
    {
      label: t("audit_drawer_tokens_out"),
      value: entry.outputTokens !== null ? entry.outputTokens.toLocaleString() : "—",
      mono: true,
    },
    {
      label: t("audit_drawer_latency"),
      value: entry.latencyMs !== null ? `${entry.latencyMs.toLocaleString()} ms` : "—",
      mono: true,
    },
    {
      label: t("audit_drawer_refs"),
      value: entry.knowledgeRefs.length > 0 ? String(entry.knowledgeRefs.length) : "—",
      mono: true,
    },
    { label: t("audit_drawer_draft"), value: entry.draftId ?? "—", mono: true },
  ];

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-50 flex justify-end"
      onClick={onClose}
      role="presentation"
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={t("audit_drawer_title")}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
        className="flex h-full w-[420px] flex-col overflow-y-auto border-divider bg-surface p-5 shadow-card [border-inline-start-width:1px]"
        style={{ insetInlineEnd: 0 }}
      >
        <div className="flex items-center gap-2">
          <h2 className="font-display text-lg italic text-p10">{t("audit_drawer_title")}</h2>
          <button
            ref={closeRef}
            type="button"
            onClick={onClose}
            aria-label={t("audit_drawer_close")}
            className="ms-auto rounded-chip p-1 text-p7 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              aria-hidden="true"
            >
              <path strokeLinecap="round" d="m4 4 8 8M12 4l-8 8" />
            </svg>
          </button>
        </div>

        {/* Event chip */}
        <div className="mt-4">
          <p className="section-label mb-1.5">{t("audit_drawer_event")}</p>
          <span
            className="rounded-chip px-2 py-0.5 font-ui text-[9px] font-semibold uppercase tracking-widest text-white"
            style={{ background: eventColorVar(entry.decisionType) }}
          >
            {eventTypeLabel(entry.decisionType)}
          </span>
        </div>

        <dl className="mt-4 space-y-3">
          {fields.map((field) => (
            <div key={field.label}>
              <dt className="section-label">{field.label}</dt>
              <dd
                className={
                  field.mono
                    ? "mt-0.5 break-all font-mono text-xs text-p9"
                    : "mt-0.5 font-body text-sm text-p9"
                }
              >
                {field.value}
              </dd>
            </div>
          ))}
        </dl>

        {/* Mis-send feedback (T086 trust downgrade input) */}
        {supportsMisSendReport(entry.decisionType) && (
          <button
            type="button"
            disabled={reportMisSend.isPending}
            onClick={() => reportMisSend.mutate({ accountId: entry.accountId })}
            className="mt-6 self-start font-ui text-[10px] font-semibold uppercase tracking-wider text-terra underline transition-colors hover:text-red focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50"
          >
            {t("audit_report_mis_send")}
          </button>
        )}
      </div>
    </div>
  );
}
