// Per-account authorization-level selector (T086, F_E3 §4.1, AI_MODES_DESIGN
// §7.2). Manual / Semi-Auto switch immediately; Full Auto stays locked until
// the account has ≥ 50 approved drafts and is then gated by E3ConfirmDialog.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import { E3ConfirmDialog } from "@/components/ai/E3ConfirmDialog";
import { EMPTY_AI_SETTINGS_PATCH } from "@/ipc/aiSettings";
import { useAccounts, useAccountAiSettings } from "@/ipc/queries/accounts";
import { useUpdateAiSettings } from "@/ipc/queries/aiProviders";
import { useApprovedDraftCount } from "@/ipc/queries/audit";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";

/** Drafts a user must approve before Full Auto unlocks (F_E3 §4.1). */
export const E3_UNLOCK_THRESHOLD = 50;

interface LevelChip {
  level: number;
  labelKey: string;
  /** Selected-state classes per the T086 token spec (p7 / slate / green). */
  activeClass: string;
}

const LEVELS: LevelChip[] = [
  { level: 1, labelKey: "auth_level_manual", activeClass: "bg-p7 text-white" },
  { level: 2, labelKey: "auth_level_semi", activeClass: "bg-slate text-white" },
  { level: 3, labelKey: "auth_level_full", activeClass: "bg-green text-white" },
];

export default function AuthLevelSection() {
  const { t } = useTranslation("aiDrafts");
  const { data: accounts } = useAccounts();

  return (
    <section aria-label={t("auth_level_section_title")}>
      <p className="section-label">{t("auth_level_section_title")}</p>
      <p className="mt-2 font-body text-sm leading-relaxed text-p8">
        {t("auth_level_section_desc")}
      </p>
      <ul className="mt-3 space-y-3">
        {(accounts ?? []).map((account) => (
          <AuthLevelRow key={account.id} account={account} />
        ))}
      </ul>
    </section>
  );
}

function AuthLevelRow({ account }: { account: Account }) {
  const { t } = useTranslation("aiDrafts");
  const { data: aiSettings } = useAccountAiSettings(account.id);
  const { data: approvedCount = 0 } = useApprovedDraftCount(account.id);
  const updateAi = useUpdateAiSettings();
  const [confirmOpen, setConfirmOpen] = useState(false);

  const currentLevel = aiSettings?.authLevel ?? account.authLevel;
  const fullAutoLocked = approvedCount < E3_UNLOCK_THRESHOLD;

  function applyLevel(level: number) {
    updateAi.mutate({
      accountId: account.id,
      params: { ...EMPTY_AI_SETTINGS_PATCH, authLevel: level },
    });
  }

  function pickLevel(level: number) {
    if (level === currentLevel) return;
    if (level === 3) {
      // Full Auto always passes through the risk confirmation (F_E3 §4.1).
      setConfirmOpen(true);
      return;
    }
    applyLevel(level);
  }

  return (
    <li className="rounded-card border border-divider bg-surface p-4 shadow-card">
      <div className="flex flex-wrap items-center gap-3">
        <span
          aria-hidden="true"
          className={cn(
            "flex h-7 w-7 shrink-0 items-center justify-center rounded-avatar font-ui text-[10px] font-semibold",
            accountColorClass((account.colorToken as AccountColorToken) ?? "team"),
          )}
        >
          {account.badgeLabel}
        </span>
        <div className="min-w-0">
          <p className="truncate font-ui text-sm font-medium text-p10">{account.displayName}</p>
          <p className="truncate font-mono text-[10px] text-p7">{account.email}</p>
        </div>

        <div
          role="radiogroup"
          aria-label={`${t("auth_level_section_title")} — ${account.displayName}`}
          className="ms-auto flex items-center gap-1.5"
        >
          {LEVELS.map(({ level, labelKey, activeClass }) => {
            const active = currentLevel === level;
            const disabled = (level === 3 && fullAutoLocked) || updateAi.isPending;
            return (
              <button
                key={level}
                type="button"
                role="radio"
                aria-checked={active}
                disabled={disabled}
                onClick={() => pickLevel(level)}
                className={cn(
                  "rounded-chip px-3 py-1.5 font-ui text-[10px] font-semibold uppercase tracking-wider transition-colors",
                  "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
                  active ? activeClass : "border border-divider text-p8 hover:bg-p4",
                  disabled && "cursor-not-allowed opacity-50",
                )}
              >
                {t(labelKey)}
              </button>
            );
          })}
        </div>
      </div>

      {/* Unlock progress — shown while Full Auto is still locked (F_E3 §4.1). */}
      {fullAutoLocked && (
        <div className="mt-3 flex items-center gap-3">
          <div
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={E3_UNLOCK_THRESHOLD}
            aria-valuenow={Math.min(approvedCount, E3_UNLOCK_THRESHOLD)}
            aria-label={t("e3_unlock_progress", { n: approvedCount, total: E3_UNLOCK_THRESHOLD })}
            className="h-1.5 w-40 overflow-hidden rounded-chip bg-p4"
          >
            <div
              className="h-full rounded-chip bg-green"
              style={{
                inlineSize: `${Math.min(100, (approvedCount / E3_UNLOCK_THRESHOLD) * 100)}%`,
              }}
            />
          </div>
          <span className="font-mono text-[10px] text-p7">
            {t("e3_unlock_progress", { n: approvedCount, total: E3_UNLOCK_THRESHOLD })}
          </span>
        </div>
      )}

      <E3ConfirmDialog
        open={confirmOpen}
        accountName={account.displayName}
        onConfirm={() => {
          setConfirmOpen(false);
          applyLevel(3);
        }}
        onCancel={() => setConfirmOpen(false)}
      />
    </li>
  );
}
