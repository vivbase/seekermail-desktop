// Batch operations card for the F4 matrix (T066, F_F4 §4.3): copy one
// capability row to all accounts, copy one account column to all of its
// capabilities, and the one-click "all Risk Checks to local" switch. Every
// operation lands as a single `batch_update_provider_matrix` call upstream.
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import { ALL_CAPABILITIES, type Capability } from "@/ipc/aiMatrix";
import { CAPABILITY_KEY } from "./MatrixGrid";

interface MatrixToolbarProps {
  accounts: Account[];
  /** Disables every action while a batch mutation is in flight. */
  busy: boolean;
  onCopyRow: (capability: Capability) => void;
  onCopyColumn: (accountId: string) => void;
  onSwitchRiskLocal: () => void;
}

export default function MatrixToolbar({
  accounts,
  busy,
  onCopyRow,
  onCopyColumn,
  onSwitchRiskLocal,
}: MatrixToolbarProps) {
  const { t } = useTranslation("aiMatrix");
  const idPrefix = useId();
  const [capability, setCapability] = useState<Capability>("DraftReply");
  const [accountId, setAccountId] = useState<string>("");
  const effectiveAccountId = accountId !== "" ? accountId : (accounts[0]?.id ?? "");

  const labelClass = "block font-ui text-[10px] uppercase tracking-wider text-p8";
  const selectClass =
    "mt-1 rounded-chip border border-divider bg-surface px-2 py-1.5 font-body text-sm text-p10";
  const buttonClass =
    "rounded-chip border border-divider px-3 py-2 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-40";

  return (
    <div className="rounded-card border border-divider bg-surface p-4 shadow-card">
      <p className="section-label">{t("matrix_batch_label")}</p>
      <div className="mt-3 flex flex-wrap items-end gap-x-4 gap-y-3">
        <div>
          <label className={labelClass} htmlFor={`${idPrefix}-capability`}>
            {t("matrix_batch_capability_label")}
          </label>
          <select
            id={`${idPrefix}-capability`}
            value={capability}
            onChange={(e) => setCapability(e.target.value as Capability)}
            className={selectClass}
          >
            {ALL_CAPABILITIES.map((cap) => (
              <option key={cap} value={cap}>
                {t(CAPABILITY_KEY[cap])}
              </option>
            ))}
          </select>
        </div>
        <button
          type="button"
          onClick={() => onCopyRow(capability)}
          disabled={busy}
          className={buttonClass}
        >
          {t("matrix_batch_copy_row")}
        </button>

        <div>
          <label className={labelClass} htmlFor={`${idPrefix}-account`}>
            {t("matrix_batch_account_label")}
          </label>
          <select
            id={`${idPrefix}-account`}
            value={effectiveAccountId}
            onChange={(e) => setAccountId(e.target.value)}
            className={selectClass}
          >
            {accounts.map((account) => (
              <option key={account.id} value={account.id}>
                {account.email}
              </option>
            ))}
          </select>
        </div>
        <button
          type="button"
          onClick={() => effectiveAccountId !== "" && onCopyColumn(effectiveAccountId)}
          disabled={busy || effectiveAccountId === ""}
          className={buttonClass}
        >
          {t("matrix_batch_copy_col")}
        </button>

        <button type="button" onClick={onSwitchRiskLocal} disabled={busy} className={buttonClass}>
          {t("matrix_batch_switch_risk_local")}
        </button>
      </div>
    </div>
  );
}
