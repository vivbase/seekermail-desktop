// E3 kill switch (T086, F_E3 §5) — sidebar status-area control. Visible only
// while at least one account runs Full Auto (authLevel 3). One press writes
// `ai.e3_paused_until = now + 24 h` to `app_settings` (the T085 pipeline reads
// it and downgrades to E2); while paused the button shows a countdown and a
// second press resumes early (writes 0).
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import { E3_PAUSE_SECS, useE3PausedUntil, useSetE3PausedUntil } from "@/ipc/queries/settings";
import { cn } from "@/lib/cn";

/** Whole hours left in the pause window (≥ 1 while active, for display). */
function pausedHoursLeft(untilUnix: number, nowUnix: number): number {
  return Math.max(1, Math.ceil((untilUnix - nowUnix) / 3600));
}

export function KillSwitch() {
  const { t } = useTranslation("aiDrafts");
  const { data: accounts } = useAccounts();
  const { data: pausedUntil = 0 } = useE3PausedUntil();
  const setPausedUntil = useSetE3PausedUntil();

  const anyFullAuto = (accounts ?? []).some((a) => a.authLevel === 3);
  if (!anyFullAuto) return null;

  const now = Math.floor(Date.now() / 1000);
  const paused = pausedUntil > now;

  function handleClick() {
    if (paused) {
      setPausedUntil.mutate(0);
    } else {
      setPausedUntil.mutate(Math.floor(Date.now() / 1000) + E3_PAUSE_SECS);
    }
  }

  return (
    <div className="px-3">
      <button
        type="button"
        onClick={handleClick}
        disabled={setPausedUntil.isPending}
        aria-pressed={paused}
        title={paused ? t("e3_kill_switch_resume_hint") : t("e3_kill_switch_pause")}
        className={cn(
          "w-full rounded-chip px-3 py-2 font-ui text-[10px] font-semibold uppercase tracking-wider transition-colors",
          "focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-50",
          paused
            ? "border border-amber text-amber hover:bg-p4"
            : "bg-amber text-white hover:opacity-90",
        )}
      >
        {paused
          ? t("e3_kill_switch_paused", { hours: pausedHoursLeft(pausedUntil, now) })
          : t("e3_kill_switch_pause")}
      </button>
    </div>
  );
}
