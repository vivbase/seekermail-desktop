// Edit-account side sheet (T017). Edits display identity (name/color/badge/role)
// and, for an auth-failed account, offers re-authorization. IPC via hooks only.
// T068 extension: the per-account AI automation level (E1 Manual / E2 Semi-Auto
// / E3 Full Auto) with a confirmation intercept before Full Auto is applied.
// Save order matters: `update_account` first (accounts.auth_level is the
// single source of truth), then `update_account_ai_settings` mirrors the level
// for the AI engine (dev/01 §account_ai_settings).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import { EMPTY_AI_SETTINGS_PATCH } from "@/ipc/aiSettings";
import { useReauthAccount, useUpdateAccount } from "@/ipc/queries/accounts";
import { useUpdateAiSettings } from "@/ipc/queries/aiProviders";
import { cn } from "@/lib/cn";

const COLOR_TOKENS = ["slate", "terra", "sage"] as const;

/** Authorization tiers (E1/E2/E3) and their aiProviders-namespace label keys. */
const AUTH_LEVELS: { level: number; labelKey: string; descKey: string }[] = [
  { level: 1, labelKey: "ai_automation_e1", descKey: "ai_automation_e1_desc" },
  { level: 2, labelKey: "ai_automation_e2", descKey: "ai_automation_e2_desc" },
  { level: 3, labelKey: "ai_automation_e3", descKey: "ai_automation_e3_desc" },
];

interface EditAccountSheetProps {
  account: Account;
  /** True when the account is in the auth-failed state (offer re-auth). */
  needsReauth?: boolean;
  onClose: () => void;
}

export default function EditAccountSheet({
  account,
  needsReauth = false,
  onClose,
}: EditAccountSheetProps) {
  const { t } = useTranslation();
  const { t: tAi } = useTranslation("aiProviders");
  const update = useUpdateAccount();
  const reauth = useReauthAccount();
  const updateAi = useUpdateAiSettings();

  const [displayName, setDisplayName] = useState(account.displayName);
  const [colorToken, setColorToken] = useState(account.colorToken);
  const [badgeLabel, setBadgeLabel] = useState(account.badgeLabel);
  const [roleType, setRoleType] = useState(account.roleType);
  const [roleDescription, setRoleDescription] = useState(account.roleDescription ?? "");
  const [password, setPassword] = useState("");
  const [authLevel, setAuthLevel] = useState(account.authLevel);
  const [confirmFullAuto, setConfirmFullAuto] = useState(false);

  /** E3 is intercepted by a confirmation dialog; E1/E2 apply directly. */
  const pickAuthLevel = (level: number) => {
    if (level === 3 && authLevel !== 3) setConfirmFullAuto(true);
    else setAuthLevel(level);
  };

  const save = () => {
    update.mutate(
      {
        accountId: account.id,
        patch: {
          displayName,
          colorToken,
          badgeLabel,
          roleType,
          roleDescription: roleDescription || null,
          authLevel: authLevel !== account.authLevel ? authLevel : null,
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
          if (authLevel === account.authLevel) {
            onClose();
            return;
          }
          // Mirror auth_level into account_ai_settings only after the
          // authoritative accounts row is updated (T073 §6 save order).
          updateAi.mutate(
            { accountId: account.id, params: { ...EMPTY_AI_SETTINGS_PATCH, authLevel } },
            { onSuccess: onClose },
          );
        },
      },
    );
  };

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-40 flex justify-end"
      role="presentation"
      onClick={onClose}
    >
      <div
        className="h-full w-full max-w-sm overflow-y-auto bg-surface p-5 shadow-card"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-display text-xl italic text-p10">{account.email}</h2>

        <div className="mt-4 space-y-3">
          <Field label={t("wizard_display_name")} value={displayName} onChange={setDisplayName} />
          <div>
            <p className="font-ui text-[10px] uppercase tracking-wider text-p8">
              {t("wizard_color")}
            </p>
            <div className="mt-1 flex gap-2">
              {COLOR_TOKENS.map((tok) => (
                <button
                  key={tok}
                  type="button"
                  aria-label={tok}
                  onClick={() => setColorToken(tok)}
                  className={`h-7 w-7 rounded-avatar bg-${tok} ${colorToken === tok ? "ring-2 ring-p9" : ""}`}
                />
              ))}
              <input
                aria-label={t("wizard_badge")}
                value={badgeLabel}
                maxLength={1}
                onChange={(e) => setBadgeLabel(e.target.value)}
                className="h-7 w-10 rounded-chip border border-divider text-center font-mono text-sm"
              />
            </div>
          </div>
          <Field label={t("role_label")} value={roleType} onChange={setRoleType} />
          <Field
            label={t("role_description")}
            value={roleDescription}
            onChange={setRoleDescription}
          />
        </div>

        {/* AI automation level (T068): E1 / E2 / E3 with an E3 intercept. */}
        <fieldset className="mt-5">
          <legend className="font-ui text-[10px] uppercase tracking-wider text-p8">
            {tAi("ai_automation_level_label")}
          </legend>
          <div className="mt-2 space-y-1" role="radiogroup">
            {AUTH_LEVELS.map(({ level, labelKey, descKey }) => (
              <button
                key={level}
                type="button"
                role="radio"
                aria-checked={authLevel === level}
                onClick={() => pickAuthLevel(level)}
                className={cn(
                  "block w-full rounded-chip border px-3 py-2 text-start transition-colors",
                  authLevel === level ? "border-p9 bg-p4" : "border-divider hover:bg-p4",
                )}
              >
                <span className="font-ui text-sm font-medium text-p10">{tAi(labelKey)}</span>
                <span className="mt-0.5 block font-body text-xs text-p8">{tAi(descKey)}</span>
              </button>
            ))}
          </div>
        </fieldset>

        {confirmFullAuto && (
          <div
            role="alertdialog"
            aria-modal="true"
            aria-label={tAi("ai_automation_e3_confirm_title")}
            className="mt-4 rounded-card border border-divider bg-p4 p-3"
          >
            <p className="font-body text-sm font-medium text-p10">
              {tAi("ai_automation_e3_confirm_title")}
            </p>
            <p className="mt-1 font-body text-xs text-p8">{tAi("ai_automation_e3_confirm")}</p>
            <div className="mt-3 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setConfirmFullAuto(false)}
                className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8"
              >
                {tAi("ai_cancel")}
              </button>
              <button
                type="button"
                onClick={() => {
                  setAuthLevel(3);
                  setConfirmFullAuto(false);
                }}
                className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white"
              >
                {tAi("ai_automation_e3_confirm_btn")}
              </button>
            </div>
          </div>
        )}

        {needsReauth && (
          <div className="mt-5 rounded-card border border-divider p-3">
            <p className="font-body text-sm text-p8">{t("acct_reauth_prompt")}</p>
            <Field
              label={t("wizard_password")}
              value={password}
              onChange={setPassword}
              type="password"
            />
            <button
              type="button"
              onClick={() =>
                reauth.mutate({ accountId: account.id, password }, { onSuccess: onClose })
              }
              className="mt-2 rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white"
            >
              {t("acct_state_auth_failed")}
            </button>
          </div>
        )}

        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8"
          >
            {t("action_cancel")}
          </button>
          <button
            type="button"
            onClick={save}
            className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white"
          >
            {t("acct_save")}
          </button>
        </div>
      </div>
    </div>
  );
}

function Field(props: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  type?: string;
}) {
  return (
    <label className="block">
      <span className="font-ui text-[10px] uppercase tracking-wider text-p8">{props.label}</span>
      <input
        type={props.type ?? "text"}
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
        className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-body text-sm text-p9"
      />
    </label>
  );
}
