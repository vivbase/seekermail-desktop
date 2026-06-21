// First-run AI activation popup. Mounted once inside AppShell; it surfaces a
// dismissible nudge — NOT a blocking gate — when an account exists but no AI
// provider is configured. Configuring AI is optional: the user can close it
// from any step (✕, "Maybe later", or the backdrop) and keep using the app.
// When they do connect, it reuses the real provider wizards and then offers to
// start the agents in Semi-Auto before closing.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { EMPTY_AI_SETTINGS_PATCH } from "@/ipc/aiSettings";
import { useAccounts } from "@/ipc/queries/accounts";
import { useConfiguredProviders, useUpdateAiSettings } from "@/ipc/queries/aiProviders";
import { useAiActivationGate } from "@/lib/aiActivationGate";
import { cn } from "@/lib/cn";
import { useActivationStore } from "@/stores/activation";
import AddCloudProviderSheet from "@/routes/settings/ai/AddCloudProviderSheet";
import AddLocalProviderSheet from "@/routes/settings/ai/AddLocalProviderSheet";

/** Semi-Auto reply tier (dev/01 §account_ai_settings; mirrors AuthLevel = 2). */
const SEMI_AUTO: number = 2;

type Phase = "gate" | "ready";
type SheetKind = "cloud" | "local" | null;

export default function AiActivationPrompt() {
  const { t } = useTranslation("activation");
  const { ready, needsActivation } = useAiActivationGate();
  const dismiss = useActivationStore((s) => s.dismiss);
  const { data: accounts } = useAccounts();
  const { data: providers } = useConfiguredProviders();
  const updateAi = useUpdateAiSettings();

  // Latch visibility: once the nudge is warranted it stays open until the user
  // closes it, so saving a provider (which clears `needsActivation`) doesn't make
  // the Semi-Auto step vanish mid-flow.
  const [visible, setVisible] = useState(false);
  const [phase, setPhase] = useState<Phase>("gate");
  const [sheet, setSheet] = useState<SheetKind>(null);
  const [activating, setActivating] = useState(false);

  useEffect(() => {
    if (ready && needsActivation) setVisible(true);
  }, [ready, needsActivation]);

  if (!visible) return null;

  // Closing dismisses for the session (so it never re-opens this run) and hides it.
  const close = () => {
    dismiss();
    setVisible(false);
  };

  const onProviderSaved = () => {
    setSheet(null);
    setPhase("ready");
  };

  const activateSemiAuto = async () => {
    setActivating(true);
    const connected = (providers ?? [])
      .filter((p) => p.provider !== "none")
      .map((p) => p.accountId);
    const ids = connected.length > 0 ? connected : (accounts ?? []).map((a) => a.id);
    try {
      for (const accountId of ids) {
        await updateAi.mutateAsync({
          accountId,
          params: { ...EMPTY_AI_SETTINGS_PATCH, authLevel: SEMI_AUTO },
        });
      }
    } finally {
      setActivating(false);
      close();
    }
  };

  return (
    <div
      className="bg-p10/50 fixed inset-0 z-30 flex items-center justify-center p-4"
      role="presentation"
      onClick={close}
    >
      <div
        className="relative w-full max-w-[480px] rounded-card bg-surface p-8 shadow-card"
        role="dialog"
        aria-modal="true"
        aria-label={phase === "gate" ? t("gate_title") : t("ready_title")}
        onClick={(e) => e.stopPropagation()}
      >
        <button
          type="button"
          onClick={close}
          aria-label={t("close")}
          className="absolute end-4 top-4 flex h-8 w-8 items-center justify-center rounded-chip text-p8 transition-colors hover:bg-p4 hover:text-p10"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" aria-hidden>
            <path
              d="M6 6l12 12M18 6L6 18"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
            />
          </svg>
        </button>

        {phase === "gate" ? (
          <section aria-label={t("gate_title")}>
            <p className="section-label text-terra">{t("eyebrow_one_last_step")}</p>
            <h1 className="mt-2 font-display text-3xl italic text-p10">{t("gate_title")}</h1>
            <p className="mt-3 font-body text-sm leading-relaxed text-p8">{t("gate_body")}</p>

            <div className="mt-6 overflow-hidden rounded-chip border border-divider">
              <StatusRow
                ok
                label={t("status_mail_label")}
                value={t("status_mail_value")}
                tag={t("status_mail_tag")}
              />
              <StatusRow
                label={t("status_ai_label")}
                value={t("status_ai_value")}
                tag={t("status_ai_tag")}
                divided
              />
            </div>

            <div className="mt-6 flex flex-col gap-2">
              <button type="button" onClick={() => setSheet("cloud")} className={primaryBtn}>
                {t("cta_add_key")}
              </button>
              <button type="button" onClick={() => setSheet("local")} className={secondaryBtn}>
                {t("cta_use_local")}
              </button>
              <button type="button" onClick={close} className={mutedBtn}>
                {t("cta_skip")}
              </button>
            </div>
          </section>
        ) : (
          <section aria-label={t("ready_title")}>
            <p className="section-label text-green">{t("ready_eyebrow")}</p>
            <h1 className="mt-2 font-display text-3xl italic text-p10">{t("ready_title")}</h1>
            <p className="mt-3 font-body text-sm leading-relaxed text-p8">{t("ready_body")}</p>

            <ul className="mt-6 space-y-2">
              <ModeRow name={t("mode_full")} desc={t("mode_full_desc")} />
              <ModeRow
                name={t("mode_semi")}
                desc={t("mode_semi_desc")}
                badge={t("mode_semi_badge")}
                highlighted
              />
              <ModeRow name={t("mode_manual")} desc={t("mode_manual_desc")} />
            </ul>

            <div className="mt-6 flex flex-col gap-2">
              <button
                type="button"
                onClick={() => void activateSemiAuto()}
                disabled={activating}
                className={cn(primaryBtn, "disabled:cursor-not-allowed disabled:opacity-50")}
              >
                {activating ? t("activating") : t("cta_activate_semi")}
              </button>
              <button type="button" onClick={close} className={mutedBtn}>
                {t("cta_choose_later")}
              </button>
            </div>
          </section>
        )}
      </div>

      {sheet === "cloud" && accounts && (
        <AddCloudProviderSheet
          accounts={accounts}
          onClose={() => setSheet(null)}
          onSaved={onProviderSaved}
        />
      )}
      {sheet === "local" && accounts && (
        <AddLocalProviderSheet
          accounts={accounts}
          onClose={() => setSheet(null)}
          onSaved={onProviderSaved}
        />
      )}
    </div>
  );
}

const primaryBtn =
  "rounded-chip bg-p9 px-5 py-2.5 font-ui text-sm font-medium uppercase tracking-wide text-white transition-colors hover:bg-p10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-p9";
const secondaryBtn =
  "rounded-chip border border-divider px-5 py-2.5 font-ui text-sm font-medium uppercase tracking-wide text-p9 transition-colors hover:bg-p4";
const mutedBtn = "font-ui text-xs uppercase tracking-wide text-p8 transition-colors hover:text-p9";

function StatusRow({
  ok = false,
  label,
  value,
  tag,
  divided = false,
}: {
  ok?: boolean;
  label: string;
  value: string;
  tag: string;
  divided?: boolean;
}) {
  return (
    <div className={cn("flex items-center gap-3 px-4 py-3", divided && "border-t border-divider")}>
      <span
        className={cn(
          "flex h-7 w-7 shrink-0 items-center justify-center rounded-chip font-mono text-sm",
          ok ? "bg-green/15 text-green" : "bg-terra/15 text-terra",
        )}
        aria-hidden
      >
        {ok ? "✓" : "!"}
      </span>
      <div className="min-w-0 flex-1">
        <p className="font-ui text-sm font-medium text-p10">{label}</p>
        <p className="font-body text-xs text-p7">{value}</p>
      </div>
      <span
        className={cn(
          "shrink-0 rounded-chip px-2 py-0.5 font-ui text-[10px] font-semibold uppercase tracking-wider",
          ok ? "bg-green/10 text-green" : "bg-terra/10 text-terra",
        )}
      >
        {tag}
      </span>
    </div>
  );
}

function ModeRow({
  name,
  desc,
  badge,
  highlighted = false,
}: {
  name: string;
  desc: string;
  badge?: string;
  highlighted?: boolean;
}) {
  return (
    <li
      className={cn(
        "rounded-chip border px-4 py-3",
        highlighted ? "border-p9 bg-p4" : "border-divider",
      )}
    >
      <div className="flex items-center gap-2">
        <span className="font-ui text-sm font-semibold text-p10">{name}</span>
        {badge && (
          <span className="rounded-chip bg-green px-2 py-0.5 font-ui text-[10px] font-semibold uppercase tracking-wider text-white">
            {badge}
          </span>
        )}
      </div>
      <p className="mt-0.5 font-body text-xs text-p8">{desc}</p>
    </li>
  );
}
