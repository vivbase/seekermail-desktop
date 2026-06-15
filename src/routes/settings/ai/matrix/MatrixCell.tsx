// One (capability, account) matrix cell (T066, F_F4 §4.2). The cell face shows
// the primary provider with a local/cloud kind dot and a backup-count badge;
// clicking opens an inline popover editor (not a route) with the primary
// selector plus an ordered backup chain of at most MAX_BACKUPS links. The
// primary≠backup rule is enforced inline here as well as by the backend.
import { useEffect, useId, useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";

import {
  MAX_BACKUPS,
  isLocalProvider,
  type MatrixCell as MatrixCellValue,
  type ProviderAssignment,
} from "@/ipc/aiMatrix";
import type { AiProvider, ConfiguredProviderInfo } from "@/ipc/aiSettings";
import { cn } from "@/lib/cn";

/** One selectable provider in the editor dropdowns, aggregated per slug. */
export interface ProviderOption {
  provider: AiProvider;
  /** Models seen for this provider across the configured accounts. */
  models: string[];
  baseUrl: string | null;
}

/** Collapse the configured-provider rows into unique dropdown options. */
export function providerOptionsFrom(providers: ConfiguredProviderInfo[]): ProviderOption[] {
  const map = new Map<AiProvider, ProviderOption>();
  for (const row of providers) {
    if (row.provider === "none") continue;
    const existing = map.get(row.provider);
    if (existing) {
      if (row.model && !existing.models.includes(row.model)) existing.models.push(row.model);
    } else {
      map.set(row.provider, {
        provider: row.provider,
        models: row.model ? [row.model] : [],
        baseUrl: row.baseUrl,
      });
    }
  }
  return [...map.values()];
}

/** Best-effort message extraction from a normalised IpcError (09 §4). */
export function errorText(e: unknown, fallback: string): string {
  if (e !== null && typeof e === "object" && "message" in e) {
    const message = (e as { message: unknown }).message;
    if (typeof message === "string" && message.length > 0) return message;
  }
  return fallback;
}

/** Draft state for one assignment row in the editor. */
interface DraftAssignment {
  provider: AiProvider | "";
  model: string;
}

interface MatrixCellProps {
  /** Localized capability row label (used in the accessible cell name). */
  capabilityLabel: string;
  /** Column identity: an account email, or the simplified "All Accounts" label. */
  accountLabel: string;
  cell: MatrixCellValue | null;
  /** Localized advisory warning for this cell, when the last save raised one. */
  warningText?: string;
  /** ≥ 4 account columns — hide the model line, keep the badge row (F_F4 §5). */
  narrow: boolean;
  options: ProviderOption[];
  /** Clearing falls back to defaults; hidden in simplified mode. */
  allowClear: boolean;
  onSave: (cell: MatrixCellValue) => Promise<void>;
  onClear: () => Promise<void>;
}

export default function MatrixCell({
  capabilityLabel,
  accountLabel,
  cell,
  warningText,
  narrow,
  options,
  allowClear,
  onSave,
  onClear,
}: MatrixCellProps) {
  const { t } = useTranslation(["aiMatrix", "aiProviders"]);
  const idPrefix = useId();
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const firstFieldRef = useRef<HTMLSelectElement>(null);

  const [open, setOpen] = useState(false);
  const [primary, setPrimary] = useState<DraftAssignment>({ provider: "", model: "" });
  const [backups, setBackups] = useState<DraftAssignment[]>([]);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const cellLabel = t("aiMatrix:matrix_cell_edit_label", {
    capability: capabilityLabel,
    account: accountLabel,
  });

  const openEditor = () => {
    setPrimary(
      cell
        ? { provider: cell.primary.provider, model: cell.primary.model }
        : { provider: "", model: "" },
    );
    setBackups(cell ? cell.backups.map((b) => ({ provider: b.provider, model: b.model })) : []);
    setSaveError(null);
    setOpen(true);
  };

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) triggerRef.current?.focus();
  };

  // Focus the first field on open; close on any click outside the cell.
  useEffect(() => {
    if (!open) return;
    firstFieldRef.current?.focus();
    const onPointerDown = (event: MouseEvent) => {
      if (!containerRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    return () => document.removeEventListener("mousedown", onPointerDown);
  }, [open]);

  const optionFor = (provider: AiProvider | "") =>
    options.find((o) => o.provider === provider) ?? null;

  const providerName = (provider: AiProvider) =>
    t(`aiProviders:ai_provider_type_${provider}`, { defaultValue: provider });

  const hasDuplicate = backups.some((b) => b.provider !== "" && b.provider === primary.provider);

  const toAssignment = (draft: DraftAssignment): ProviderAssignment => ({
    provider: draft.provider as AiProvider,
    model: draft.model.trim(),
    baseUrl: optionFor(draft.provider)?.baseUrl ?? null,
  });

  const save = async () => {
    if (primary.provider === "") {
      setSaveError(t("aiMatrix:matrix_error_primary_required"));
      return;
    }
    if (hasDuplicate) return;
    setBusy(true);
    setSaveError(null);
    try {
      await onSave({
        primary: toAssignment(primary),
        backups: backups.filter((b) => b.provider !== "").map(toAssignment),
      });
      close(true);
    } catch (e) {
      setSaveError(errorText(e, t("aiMatrix:matrix_save_failed")));
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    setSaveError(null);
    try {
      await onClear();
      close(true);
    } catch (e) {
      setSaveError(errorText(e, t("aiMatrix:matrix_save_failed")));
    } finally {
      setBusy(false);
    }
  };

  const setBackup = (index: number, patch: Partial<DraftAssignment>) =>
    setBackups((prev) => prev.map((b, i) => (i === index ? { ...b, ...patch } : b)));

  const fieldLabelClass = "mt-2 block font-ui text-[10px] uppercase tracking-wider text-p8";
  const fieldClass =
    "mt-1 w-full rounded-chip border border-divider bg-surface px-2 py-1.5 font-body text-sm text-p10";

  const assignmentFields = (
    draft: DraftAssignment,
    providerId: string,
    providerLabel: string,
    modelId: string,
    modelLabel: string,
    onChange: (patch: Partial<DraftAssignment>) => void,
    fieldRef?: RefObject<HTMLSelectElement>,
  ) => (
    <>
      <label className={fieldLabelClass} htmlFor={providerId}>
        {providerLabel}
      </label>
      <select
        id={providerId}
        ref={fieldRef}
        value={draft.provider}
        onChange={(e) => {
          const provider = e.target.value as AiProvider | "";
          onChange({ provider, model: optionFor(provider)?.models[0] ?? "" });
        }}
        className={fieldClass}
      >
        <option value="">{t("aiMatrix:matrix_cell_select_provider")}</option>
        {options.map((option) => (
          <option key={option.provider} value={option.provider}>
            {providerName(option.provider)}
          </option>
        ))}
      </select>
      <label className={fieldLabelClass} htmlFor={modelId}>
        {modelLabel}
      </label>
      <input
        id={modelId}
        type="text"
        value={draft.model}
        onChange={(e) => onChange({ model: e.target.value })}
        className={fieldClass}
      />
    </>
  );

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        ref={triggerRef}
        onClick={() => (open ? close(false) : openEditor())}
        aria-label={cellLabel}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={warningText}
        className={cn(
          "flex min-h-12 w-full flex-col items-start justify-center gap-0.5 rounded-chip border border-divider bg-surface p-2 text-start transition-colors hover:bg-p4",
          warningText && "border-s-4 border-s-amber",
        )}
      >
        {cell ? (
          <>
            <span className="flex w-full items-center gap-1.5">
              <span
                aria-hidden
                className={cn(
                  "h-2 w-2 shrink-0 rounded-avatar",
                  isLocalProvider(cell.primary.provider) ? "bg-green" : "bg-slate",
                )}
              />
              <span className="sr-only">
                {isLocalProvider(cell.primary.provider)
                  ? t("aiMatrix:matrix_local_badge")
                  : t("aiMatrix:matrix_cloud_badge")}
              </span>
              <span className="truncate font-ui text-xs text-p10">
                {providerName(cell.primary.provider)}
              </span>
              {cell.backups.length > 0 && (
                <span className="ms-auto shrink-0 rounded-chip bg-p4 px-1.5 py-0.5 font-mono text-[10px] text-p8">
                  {t("aiMatrix:matrix_cell_backups_badge", { count: cell.backups.length })}
                </span>
              )}
            </span>
            {!narrow && cell.primary.model !== "" && (
              <span className="w-full truncate font-mono text-[11px] text-p8">
                {cell.primary.model}
              </span>
            )}
          </>
        ) : (
          <span className="font-body text-xs italic text-p7">
            {t("aiMatrix:matrix_cell_unassigned")}
          </span>
        )}
      </button>

      {open && (
        <div
          role="dialog"
          aria-label={cellLabel}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              close(true);
            }
          }}
          className="absolute start-0 top-full z-20 mt-1 w-72 rounded-card border border-divider bg-surface p-4 shadow-card"
        >
          {options.length === 0 ? (
            <p className="font-body text-xs text-p8">{t("aiMatrix:matrix_no_providers_hint")}</p>
          ) : (
            <>
              <p className="section-label">{t("aiMatrix:matrix_cell_primary")}</p>
              {assignmentFields(
                primary,
                `${idPrefix}-primary-provider`,
                t("aiMatrix:matrix_cell_primary_provider"),
                `${idPrefix}-primary-model`,
                t("aiMatrix:matrix_cell_primary_model"),
                (patch) => setPrimary((prev) => ({ ...prev, ...patch })),
                firstFieldRef,
              )}
              <p className="mt-1 font-body text-[11px] text-p7">
                {t("aiMatrix:matrix_cell_model_hint")}
              </p>

              {backups.map((backup, index) => (
                <div key={index} className="mt-3 border-t border-divider pt-3">
                  <div className="flex items-center justify-between">
                    <p className="section-label">
                      {t("aiMatrix:matrix_cell_backup", { index: index + 1 })}
                    </p>
                    <button
                      type="button"
                      onClick={() => setBackups((prev) => prev.filter((_, i) => i !== index))}
                      className="font-ui text-[10px] uppercase tracking-wider text-red transition-colors hover:text-p10"
                    >
                      {t("aiMatrix:matrix_cell_remove_backup", { index: index + 1 })}
                    </button>
                  </div>
                  {assignmentFields(
                    backup,
                    `${idPrefix}-backup-${index}-provider`,
                    t("aiMatrix:matrix_cell_backup_provider", { index: index + 1 }),
                    `${idPrefix}-backup-${index}-model`,
                    t("aiMatrix:matrix_cell_backup_model", { index: index + 1 }),
                    (patch) => setBackup(index, patch),
                  )}
                  {backup.provider !== "" && backup.provider === primary.provider && (
                    <p role="alert" className="mt-1 font-body text-xs text-red">
                      {t("aiMatrix:matrix_error_duplicate_provider")}
                    </p>
                  )}
                </div>
              ))}

              <button
                type="button"
                onClick={() => setBackups((prev) => [...prev, { provider: "", model: "" }])}
                disabled={backups.length >= MAX_BACKUPS}
                className="mt-3 rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40"
              >
                {t("aiMatrix:matrix_cell_add_backup")}
              </button>
              {backups.length >= MAX_BACKUPS && (
                <p className="mt-1 font-body text-[11px] text-p7">
                  {t("aiMatrix:matrix_error_max_backups")}
                </p>
              )}

              {saveError && (
                <p role="alert" className="mt-3 font-body text-xs text-red">
                  {saveError}
                </p>
              )}

              <div className="mt-4 flex items-center justify-end gap-2">
                <button
                  type="button"
                  onClick={() => close(true)}
                  className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 transition-colors hover:bg-p4"
                >
                  {t("aiMatrix:matrix_cell_cancel")}
                </button>
                {allowClear && cell && (
                  <button
                    type="button"
                    onClick={clear}
                    disabled={busy}
                    className="rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-red transition-colors hover:bg-p4 disabled:opacity-40"
                  >
                    {t("aiMatrix:matrix_cell_clear")}
                  </button>
                )}
                <button
                  type="button"
                  onClick={save}
                  disabled={busy || hasDuplicate}
                  className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 disabled:opacity-40"
                >
                  {busy ? t("aiMatrix:matrix_cell_saving") : t("aiMatrix:matrix_cell_save")}
                </button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
