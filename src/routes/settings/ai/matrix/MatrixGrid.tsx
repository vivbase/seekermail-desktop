// Capability × account table layout for the F4 matrix (T066, F_F4 §3, §5).
// Pure CSS grid — no table library. Fine mode renders one column per active
// account (narrow headers from 4 accounts up); simplified mode collapses to a
// single shared "All Accounts" column backed by the first account's matrix.
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import {
  ALL_CAPABILITIES,
  matrixCellOf,
  type Capability,
  type CapabilityMatrix,
  type MatrixCell as MatrixCellValue,
} from "@/ipc/aiMatrix";
import type { ConfiguredProviderInfo } from "@/ipc/aiSettings";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import MatrixCell, { providerOptionsFrom } from "./MatrixCell";

/** Capability → row-label i18n key (aiMatrix namespace), in spec row order. */
export const CAPABILITY_KEY: Record<Capability, string> = {
  DraftReply: "matrix_capability_draft_reply",
  RiskReason: "matrix_capability_risk_check",
  Summarize: "matrix_capability_summarize",
  StyleProfile: "matrix_capability_style_profile",
};

export type MatrixMode = "fine" | "simplified";

interface MatrixGridProps {
  accounts: Account[];
  mode: MatrixMode;
  /** Matrix per account id; `undefined` while that column is still loading. */
  matrices: Record<string, CapabilityMatrix | undefined>;
  /** Localized warning text per `${accountId}:${capability}` cell. */
  warningTexts: Record<string, string>;
  providers: ConfiguredProviderInfo[];
  onSaveCell: (accountId: string, capability: Capability, cell: MatrixCellValue) => Promise<void>;
  onClearCell: (accountId: string, capability: Capability) => Promise<void>;
}

export default function MatrixGrid({
  accounts,
  mode,
  matrices,
  warningTexts,
  providers,
  onSaveCell,
  onClearCell,
}: MatrixGridProps) {
  const { t } = useTranslation("aiMatrix");
  const options = providerOptionsFrom(providers);

  // Simplified mode: one shared column, seeded from the first account (F_F4 §5).
  const columns = mode === "simplified" ? accounts.slice(0, 1) : accounts;
  const narrow = mode === "fine" && accounts.length >= 4;
  const columnLabel = (account: Account) =>
    mode === "simplified" ? t("matrix_col_all_accounts") : account.email;

  return (
    <div
      role="table"
      aria-label={t("matrix_page_title")}
      className="grid items-stretch gap-2"
      style={{
        gridTemplateColumns: `minmax(88px, auto) repeat(${columns.length}, minmax(${
          narrow ? "60px" : "120px"
        }, 1fr))`,
      }}
    >
      <div role="row" className="contents">
        <div role="columnheader" aria-label={t("matrix_capability_column_label")} />
        {columns.map((account) => (
          <div role="columnheader" key={account.id} className="flex min-w-0 items-center gap-2">
            {mode === "simplified" ? (
              <span className="font-ui text-xs uppercase tracking-wider text-p9">
                {t("matrix_col_all_accounts")}
              </span>
            ) : (
              <>
                <span
                  aria-hidden
                  className={cn(
                    "flex h-6 w-6 shrink-0 items-center justify-center rounded-avatar font-ui text-[10px]",
                    accountColorClass(account.colorToken as AccountColorToken),
                  )}
                >
                  {account.badgeLabel}
                </span>
                {narrow ? (
                  <span className="sr-only">{account.email}</span>
                ) : (
                  <span className="truncate font-mono text-xs text-p8" title={account.email}>
                    {account.email}
                  </span>
                )}
              </>
            )}
          </div>
        ))}
      </div>

      {ALL_CAPABILITIES.map((capability) => (
        <div role="row" className="contents" key={capability}>
          <div role="rowheader" className="flex items-center">
            <span className="section-label">{t(CAPABILITY_KEY[capability])}</span>
          </div>
          {columns.map((account) => (
            <div role="cell" key={account.id} className="min-w-0">
              <MatrixCell
                capabilityLabel={t(CAPABILITY_KEY[capability])}
                accountLabel={columnLabel(account)}
                cell={matrixCellOf(matrices[account.id], capability)}
                warningText={warningTexts[`${account.id}:${capability}`]}
                narrow={narrow}
                options={options}
                allowClear={mode === "fine"}
                onSave={(cell) => onSaveCell(account.id, capability, cell)}
                onClear={() => onClearCell(account.id, capability)}
              />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}
