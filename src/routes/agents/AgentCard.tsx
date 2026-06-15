// Per-account "digital employee" card (T073). Shows the account identity
// (badge / display name / email), the RoleEditor form, the read-only AI
// provider status row, and an independent Save action. Save order matters:
// `update_account` first (single source of truth for auth_level), then
// `update_account_ai_settings` mirrors the level for the AI engine (dev/01
// §account_ai_settings). Either failure surfaces as an inline error.
import { useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";
import type { Account, IpcError } from "@shared/bindings";

import { normalizeIpcError } from "@/ipc/client";
import { uxForCode } from "@/ipc/errors";
import { EMPTY_AI_SETTINGS_PATCH } from "@/ipc/aiSettings";
import {
  useAccountAiSettings,
  useSetPrimaryAccount,
  useUpdateAccount,
  useUpdateAccountAiSettings,
} from "@/ipc/queries/accounts";
import SetPrimaryDialog from "@/components/agent/SetPrimaryDialog";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import RoleEditor, { MAX_ROLE_DESCRIPTION_CHARS } from "./RoleEditor";

/** Decorative gold star glyph for the primary account (not translatable copy). */
const PRIMARY_STAR = "★";

/** Card accent per role_type (T073 §3.2; account color coding, root CLAUDE.md). */
const ROLE_ACCENT_VAR: Record<string, string> = {
  legal: "var(--terra)",
  work: "var(--slate)",
  personal: "var(--sage)",
  sales: "var(--amber)",
  custom: "var(--p9)",
};

interface AgentCardProps {
  account: Account;
  /** Bubble the success toast up to the page (one toast region for all cards). */
  onSaved: (message: string) => void;
}

export default function AgentCard({ account, onSaved }: AgentCardProps) {
  const { t } = useTranslation(["agents", "errors"]);

  const aiSettings = useAccountAiSettings(account.id);
  const updateAccount = useUpdateAccount();
  const updateAiSettings = useUpdateAccountAiSettings();
  const setPrimary = useSetPrimaryAccount();

  // Form dirty state lives inside the card — never shared globally (T073 §3.3).
  const [roleType, setRoleType] = useState(account.roleType);
  const [roleDescription, setRoleDescription] = useState(account.roleDescription ?? "");
  const [authLevel, setAuthLevel] = useState(account.authLevel);
  const [error, setError] = useState<IpcError | null>(null);
  // T091: primary-account promotion is gated behind a confirmation dialog.
  const [showPrimaryDialog, setShowPrimaryDialog] = useState(false);

  const confirmSetPrimary = () => {
    setError(null);
    setPrimary.mutate(account.id, {
      onSuccess: () => {
        setShowPrimaryDialog(false);
        onSaved(t("agents:set_primary_success"));
      },
      onError: (e) => {
        setShowPrimaryDialog(false);
        setError(normalizeIpcError(e));
      },
    });
  };

  const saving = updateAccount.isPending || updateAiSettings.isPending;
  const descOverLimit = roleDescription.length > MAX_ROLE_DESCRIPTION_CHARS;
  const dirty =
    roleType !== account.roleType ||
    roleDescription !== (account.roleDescription ?? "") ||
    authLevel !== account.authLevel;

  const accent = ROLE_ACCENT_VAR[roleType] ?? ROLE_ACCENT_VAR.custom;

  const save = () => {
    setError(null);
    updateAccount.mutate(
      {
        accountId: account.id,
        patch: {
          displayName: null,
          colorToken: null,
          badgeLabel: null,
          roleType,
          roleDescription: roleDescription.trim() ? roleDescription : null,
          authLevel,
          isActive: null,
          isPrimary: null,
          syncIntervalSecs: null,
          imapHost: null,
          imapPort: null,
          smtpHost: null,
          smtpPort: null,
        },
      },
      {
        onSuccess: () => {
          // Mirror auth_level into account_ai_settings only after the
          // authoritative accounts row is updated (T073 §6 save order).
          updateAiSettings.mutate(
            { accountId: account.id, params: { ...EMPTY_AI_SETTINGS_PATCH, authLevel } },
            {
              onSuccess: () => onSaved(t("agents:agents_save_success")),
              onError: (e) => setError(normalizeIpcError(e)),
            },
          );
        },
        onError: (e) => setError(normalizeIpcError(e)),
      },
    );
  };

  const provider = aiSettings.data?.aiProvider ?? "none";
  const providerText =
    provider === "none"
      ? t("agents:agents_ai_provider_none")
      : [t(`agents:ai_provider_${provider}`), aiSettings.data?.aiModel].filter(Boolean).join(" · ");

  return (
    <article
      className="rounded-card border border-divider bg-surface p-5 shadow-card [border-inline-start-color:var(--agent-accent)] [border-inline-start-width:3px]"
      style={{ "--agent-accent": accent } as CSSProperties}
      aria-label={account.displayName}
    >
      {/* Identity header (read-only) */}
      <header className="flex items-center gap-3">
        <span
          aria-hidden
          className={cn(
            "flex h-9 w-9 shrink-0 items-center justify-center rounded-avatar font-ui text-sm",
            accountColorClass(account.colorToken as AccountColorToken),
          )}
        >
          {account.badgeLabel}
        </span>
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 font-body text-sm font-medium text-p10">
            <span className="truncate">{account.displayName}</span>
            {account.isPrimary && (
              <span
                aria-label={t("agents:primary_account_badge")}
                title={t("agents:primary_account_tooltip")}
                className="shrink-0 font-ui text-amber"
              >
                {PRIMARY_STAR}
              </span>
            )}
          </p>
          <p className="truncate font-mono text-xs text-p8">{account.email}</p>
        </div>
        {account.isPrimary ? (
          <span
            title={t("agents:primary_account_tooltip")}
            className="shrink-0 rounded-chip bg-p4 px-2 py-1 font-ui text-[10px] uppercase tracking-wider text-p8"
          >
            {t("agents:primary_account_badge")}
          </span>
        ) : (
          <button
            type="button"
            onClick={() => setShowPrimaryDialog(true)}
            className="shrink-0 rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
          >
            {t("agents:set_as_primary")}
          </button>
        )}
      </header>

      <div className="mt-4">
        <RoleEditor
          accountId={account.id}
          roleType={roleType}
          onRoleTypeChange={setRoleType}
          roleDescription={roleDescription}
          onRoleDescriptionChange={setRoleDescription}
          authLevel={authLevel}
          onAuthLevelChange={setAuthLevel}
          disabled={saving}
        />
      </div>

      {/* AI provider status (read-only; configured under Settings → AI) */}
      <div className="mt-4 flex items-center justify-between border-t border-divider pt-3">
        <div className="min-w-0">
          <p className="font-ui text-[10px] uppercase tracking-wider text-p8">
            {t("agents:agents_ai_provider_label")}
          </p>
          <p className="mt-0.5 truncate font-body text-sm text-p9">
            {aiSettings.isLoading ? "…" : providerText}
          </p>
        </div>
        <Link
          to="/settings/ai"
          className="shrink-0 rounded-chip border border-divider px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p9 transition-colors hover:bg-p4"
        >
          {t("agents:agents_configure_provider")}
        </Link>
      </div>

      {/* Inline error + save */}
      {error && (
        <p role="alert" className="mt-3 rounded-chip bg-p4 px-3 py-2 font-body text-xs text-red">
          {t(`errors:${uxForCode(error.code).messageKey}`)}
        </p>
      )}
      <div className="mt-4 flex justify-end">
        <button
          type="button"
          onClick={save}
          disabled={!dirty || saving || descOverLimit}
          className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-xs uppercase tracking-wider text-white transition-colors hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {saving ? t("agents:agents_saving") : t("agents:agents_save_btn")}
        </button>
      </div>

      <SetPrimaryDialog
        open={showPrimaryDialog}
        accountName={account.displayName}
        accountEmail={account.email}
        pending={setPrimary.isPending}
        onConfirm={confirmSetPrimary}
        onCancel={() => setShowPrimaryDialog(false)}
      />
    </article>
  );
}
