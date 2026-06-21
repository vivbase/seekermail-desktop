// Global AI master switch (T067, F_F5 §4.5) — the Settings → AI control that
// disables every AI capability across all accounts for 24 h / 48 h / permanently,
// or restores it. Distinct from the per-account authorization levels and the
// sidebar E3 pause: this is the app-wide kill switch backed by `set_ai_disabled`,
// which the F5 fallback router honors for every AI call.
import { useTranslation } from "react-i18next";

import {
  AI_DISABLE_24H_SECS,
  AI_DISABLE_48H_SECS,
  AI_DISABLE_PERMANENT_UNTIL,
  isPermanentDisable,
  useAiDisabledUntil,
  useSetAiDisabled,
} from "@/ipc/queries/settings";

/** Whole hours left in the disable window (≥ 1 while active, for display). */
function disabledHoursLeft(untilUnix: number, nowUnix: number): number {
  return Math.max(1, Math.ceil((untilUnix - nowUnix) / 3600));
}

export default function AiMasterSwitchSection() {
  const { t } = useTranslation("aiProviders");
  const { data: disabledUntil = 0 } = useAiDisabledUntil();
  const setAiDisabled = useSetAiDisabled();

  const now = Math.floor(Date.now() / 1000);
  const disabled = disabledUntil > now;
  const permanent = disabled && isPermanentDisable(disabledUntil);
  const busy = setAiDisabled.isPending;

  const apply = (untilUnix: number | null) => setAiDisabled.mutate(untilUnix);

  return (
    <section>
      <p className="section-label">{t("ai_master_switch_label")}</p>
      <div className="mt-2 rounded-card border border-divider bg-surface p-5">
        <p className="font-body text-sm leading-relaxed text-p8">{t("ai_master_switch_desc")}</p>

        {disabled ? (
          <div className="mt-4 space-y-3">
            <p
              role="status"
              className="flex items-center gap-2 font-ui text-xs uppercase tracking-wider text-terra"
            >
              <span aria-hidden className="inline-block h-2 w-2 rounded-full bg-terra" />
              {permanent
                ? t("ai_master_switch_disabled_permanent")
                : t("ai_master_switch_paused_hours", {
                    hours: disabledHoursLeft(disabledUntil, now),
                  })}
            </p>
            <button
              type="button"
              onClick={() => apply(null)}
              disabled={busy}
              className="rounded-chip bg-green px-4 py-2 font-ui text-xs uppercase tracking-wider text-white transition-opacity hover:opacity-90 disabled:opacity-50"
            >
              {busy ? t("ai_master_switch_working") : t("ai_master_switch_resume")}
            </button>
          </div>
        ) : (
          <div className="mt-4 space-y-3">
            <p
              role="status"
              className="flex items-center gap-2 font-ui text-xs uppercase tracking-wider text-green"
            >
              <span aria-hidden className="inline-block h-2 w-2 rounded-full bg-green" />
              {t("ai_master_switch_active")}
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => apply(now + AI_DISABLE_24H_SECS)}
                disabled={busy}
                className="rounded-chip border border-divider px-4 py-2 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-50"
              >
                {t("ai_master_switch_pause_24h")}
              </button>
              <button
                type="button"
                onClick={() => apply(now + AI_DISABLE_48H_SECS)}
                disabled={busy}
                className="rounded-chip border border-divider px-4 py-2 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4 disabled:opacity-50"
              >
                {t("ai_master_switch_pause_48h")}
              </button>
              <button
                type="button"
                onClick={() => apply(AI_DISABLE_PERMANENT_UNTIL)}
                disabled={busy}
                className="rounded-chip border border-terra px-4 py-2 font-ui text-xs uppercase tracking-wider text-terra transition-colors hover:bg-p4 disabled:opacity-50"
              >
                {t("ai_master_switch_disable_permanent")}
              </button>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
