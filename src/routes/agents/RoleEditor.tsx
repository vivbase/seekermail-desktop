// Role + persona form (T073): role_type select, role_description textarea with
// a soft 500-char limit, and the three-tier auth_level segmented control.
// Selecting Full Auto (level 3) is intercepted by FullAutoConfirmDialog before
// the value is written to form state (AI_MODES_DESIGN §7.4). Pure controlled
// component — the owning AgentCard holds the dirty state and persists it.
import { useState } from "react";
import { useTranslation } from "react-i18next";

import FullAutoConfirmDialog from "@/components/agent/FullAutoConfirmDialog";
import { cn } from "@/lib/cn";

/** Soft front-end limit for `accounts.role_description` (T073 §3.2). */
export const MAX_ROLE_DESCRIPTION_CHARS = 500;

/** The selectable `accounts.role_type` values (dev/01 §accounts). */
export const ROLE_TYPES = ["legal", "work", "personal", "sales", "custom"] as const;

const AUTH_LEVELS = [
  { value: 1, labelKey: "agents_auth_manual", descKey: "agents_auth_manual_desc" },
  { value: 2, labelKey: "agents_auth_semi", descKey: "agents_auth_semi_desc" },
  { value: 3, labelKey: "agents_auth_full_auto", descKey: "agents_auth_full_auto_desc" },
] as const;

interface RoleEditorProps {
  /** Unique per card — keys the form controls to their labels. */
  accountId: string;
  roleType: string;
  onRoleTypeChange: (next: string) => void;
  roleDescription: string;
  onRoleDescriptionChange: (next: string) => void;
  authLevel: number;
  onAuthLevelChange: (next: number) => void;
  /** True while the owning card is saving — locks every control. */
  disabled: boolean;
}

export default function RoleEditor({
  accountId,
  roleType,
  onRoleTypeChange,
  roleDescription,
  onRoleDescriptionChange,
  authLevel,
  onAuthLevelChange,
  disabled,
}: RoleEditorProps) {
  const { t } = useTranslation("agents");
  const [confirmFullAuto, setConfirmFullAuto] = useState(false);

  const descOverLimit = roleDescription.length > MAX_ROLE_DESCRIPTION_CHARS;
  const roleTypeId = `role-type-${accountId}`;
  const roleDescId = `role-desc-${accountId}`;

  const selectAuthLevel = (value: number) => {
    if (value === authLevel) return;
    // Switching INTO Full Auto requires explicit confirmation first; the form
    // value only changes after the dialog's Confirm (T073 §3.4).
    if (value === 3) {
      setConfirmFullAuto(true);
      return;
    }
    onAuthLevelChange(value);
  };

  return (
    <div className="space-y-4">
      {/* Role type */}
      <div>
        <label
          htmlFor={roleTypeId}
          className="font-ui text-[10px] uppercase tracking-wider text-p8"
        >
          {t("agents_role_type_label")}
        </label>
        <select
          id={roleTypeId}
          value={roleType}
          disabled={disabled}
          onChange={(e) => onRoleTypeChange(e.target.value)}
          className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-body text-sm text-p9 disabled:cursor-not-allowed disabled:opacity-60"
        >
          {ROLE_TYPES.map((value) => (
            <option key={value} value={value}>
              {t(`role_type_${value}`)}
            </option>
          ))}
        </select>
      </div>

      {/* Role description */}
      <div>
        <div className="flex items-baseline justify-between">
          <label
            htmlFor={roleDescId}
            className="font-ui text-[10px] uppercase tracking-wider text-p8"
          >
            {t("agents_role_description_label")}
          </label>
          <span
            aria-live="polite"
            className={cn("font-mono text-[10px]", descOverLimit ? "text-terra" : "text-p8")}
          >
            {roleDescription.length}/{MAX_ROLE_DESCRIPTION_CHARS}
          </span>
        </div>
        <textarea
          id={roleDescId}
          rows={3}
          value={roleDescription}
          disabled={disabled}
          placeholder={t("agents_role_description_ph")}
          aria-invalid={descOverLimit || undefined}
          onChange={(e) => onRoleDescriptionChange(e.target.value)}
          className={cn(
            "mt-1 w-full resize-y rounded-chip border bg-surface px-3 py-2 font-body text-sm text-p9 disabled:cursor-not-allowed disabled:opacity-60",
            descOverLimit ? "border-terra" : "border-divider",
          )}
        />
        {descOverLimit && (
          <p role="alert" className="mt-1 font-body text-xs text-terra">
            {t("agents_role_description_over_limit", { max: MAX_ROLE_DESCRIPTION_CHARS })}
          </p>
        )}
      </div>

      {/* Authorization level */}
      <div>
        <p className="font-ui text-[10px] uppercase tracking-wider text-p8">
          {t("agents_auth_level_label")}
        </p>
        <div
          role="radiogroup"
          aria-label={t("agents_auth_level_label")}
          className="mt-1 inline-flex overflow-hidden rounded-chip border border-divider"
        >
          {AUTH_LEVELS.map((level) => (
            <button
              key={level.value}
              type="button"
              role="radio"
              aria-checked={authLevel === level.value}
              disabled={disabled}
              onClick={() => selectAuthLevel(level.value)}
              className={cn(
                "border-divider px-3 py-1.5 font-ui text-xs transition-colors [border-inline-end-width:1px] last:[border-inline-end-width:0] disabled:cursor-not-allowed disabled:opacity-60",
                authLevel === level.value
                  ? "bg-p9 font-medium text-p1"
                  : "bg-surface text-p9 hover:bg-p4",
              )}
            >
              {t(level.labelKey)}
            </button>
          ))}
        </div>
        <p className="mt-2 font-body text-xs leading-relaxed text-p8">
          {t(AUTH_LEVELS.find((l) => l.value === authLevel)?.descKey ?? "agents_auth_manual_desc")}
        </p>
        {authLevel === 3 && (
          <p
            role="note"
            className="mt-2 rounded-chip border-amber bg-p4 px-3 py-2 font-body text-xs leading-relaxed text-p9 [border-inline-start-width:3px]"
          >
            {t("agents_full_auto_warning")}
          </p>
        )}
      </div>

      <FullAutoConfirmDialog
        open={confirmFullAuto}
        onConfirm={() => {
          setConfirmFullAuto(false);
          onAuthLevelChange(3);
        }}
        onCancel={() => setConfirmFullAuto(false)}
      />
    </div>
  );
}
