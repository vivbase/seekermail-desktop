// Account filter chip row for the search panel (T113). An "All Accounts" chip
// plus one chip per active account (max 6, then a "+N more" indicator). Selection
// lives in the UI store (`searchAccountFilter`); chip colours come from each
// account's `colorToken` via the `--chip-color` CSS variable — no hardcoded hex
// (dev/07 §7). Reuses the T048 account-colour convention.
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import { useUi } from "@/stores/ui";
import { cn } from "@/lib/cn";

/** Design cap on visible account chips (T113 §3a). */
const MAX_CHIPS = 6;

/** Map an account colour token to its design-system CSS variable. */
function chipColorVar(colorToken: string): string {
  return `var(--${colorToken})`;
}

export function AccountFilterBar() {
  const { t } = useTranslation("search");
  const { data: accounts } = useAccounts();
  const selected = useUi((s) => s.searchAccountFilter);
  const toggle = useUi((s) => s.toggleSearchAccount);
  const clear = useUi((s) => s.clearSearchAccountFilter);

  const active = (accounts ?? []).filter((a) => a.isActive);
  // Nothing to filter across with a single account.
  if (active.length <= 1) return null;

  const shown = active.slice(0, MAX_CHIPS);
  const overflow = active.length - shown.length;
  const allSelected = selected.length === 0;

  return (
    <div
      role="group"
      aria-label={t("search_filter_accounts_label")}
      className="flex flex-wrap items-center gap-1.5 border-b border-divider px-4 py-2"
    >
      <button
        type="button"
        data-selected={allSelected}
        onClick={clear}
        className={cn(
          "rounded-chip border px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider transition-colors",
          allSelected ? "border-p9 bg-p9 text-white" : "border-divider text-p8 hover:bg-p4",
        )}
      >
        {t("search_filter_all_accounts")}
      </button>

      {shown.map((account) => {
        const isSel = selected.includes(account.id);
        const color = chipColorVar(account.colorToken);
        return (
          <button
            key={account.id}
            type="button"
            data-selected={isSel}
            data-color-token={account.colorToken}
            onClick={() => toggle(account.id)}
            title={account.displayName}
            style={
              {
                "--chip-color": color,
                ...(isSel
                  ? {
                      backgroundColor: `color-mix(in srgb, ${color} 15%, transparent)`,
                      borderColor: color,
                    }
                  : {}),
              } as React.CSSProperties
            }
            className={cn(
              "flex items-center gap-1.5 rounded-chip border px-2 py-1 font-ui text-[10px] transition-colors",
              isSel ? "text-p10" : "border-divider text-p8 hover:bg-p4",
            )}
          >
            <span
              aria-hidden
              className="flex h-3.5 w-3.5 items-center justify-center rounded-full text-[8px] font-semibold text-white"
              style={{ backgroundColor: color }}
            >
              {account.badgeLabel}
            </span>
            <span className="max-w-[10ch] truncate">{account.displayName}</span>
          </button>
        );
      })}

      {overflow > 0 && (
        <span className="rounded-chip border border-divider px-2 py-1 font-ui text-[10px] text-p7">
          {t("search_accounts_overflow", { n: overflow })}
        </span>
      )}
    </div>
  );
}
