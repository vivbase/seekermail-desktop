// Privacy & Security settings (T051). Two three-level policies — tracking
// protection (B2 ruleset) and remote-image loading (B1 isolation) — persisted to
// `app_settings` through `apply_privacy_policy` and applied by the Rust pipeline
// on its next mail-processing pass. Immediate-save (F_H1 §4.3); Reset to
// Defaults sits behind an explicit confirm dialog (never `confirm()`).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ImagePolicy, TrackerPolicy } from "@shared/bindings";

import {
  DEFAULT_REMOTE_IMAGE_POLICY,
  DEFAULT_TRACKER_POLICY,
  usePrivacySettings,
  useSetPrivacySettings,
} from "@/ipc/queries/settings";
import { cn } from "@/lib/cn";

// ── Option tables ─────────────────────────────────────────────────────────────

const TRACKER_OPTIONS: { value: TrackerPolicy; labelKey: string; descKey: string }[] = [
  {
    value: "block_all",
    labelKey: "privacy_tracker_block_all",
    descKey: "privacy_tracker_block_all_desc",
  },
  {
    value: "block_known",
    labelKey: "privacy_tracker_block_known",
    descKey: "privacy_tracker_block_known_desc",
  },
  {
    value: "allow_all",
    labelKey: "privacy_tracker_allow_all",
    descKey: "privacy_tracker_allow_all_desc",
  },
];

const IMAGE_OPTIONS: { value: ImagePolicy; labelKey: string; descKey: string }[] = [
  {
    value: "block_all",
    labelKey: "privacy_image_block_all",
    descKey: "privacy_image_block_all_desc",
  },
  {
    value: "trusted_only",
    labelKey: "privacy_image_trusted_only",
    descKey: "privacy_image_trusted_only_desc",
  },
  {
    value: "allow_all",
    labelKey: "privacy_image_allow_all",
    descKey: "privacy_image_allow_all_desc",
  },
];

// ── Segmented control ─────────────────────────────────────────────────────────

function SegmentedControl<V extends string>({
  label,
  options,
  value,
  onChange,
  t,
}: {
  label: string;
  options: { value: V; labelKey: string; descKey: string }[];
  value: V;
  onChange: (next: V) => void;
  t: (key: string) => string;
}) {
  const selected = options.find((o) => o.value === value);
  return (
    <div className="rounded-card border border-divider bg-surface px-4 py-4">
      <p className="font-ui text-sm font-medium text-p9">{label}</p>
      <div
        role="radiogroup"
        aria-label={label}
        className="mt-3 inline-flex overflow-hidden rounded-chip border border-divider"
      >
        {options.map((opt) => (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={value === opt.value}
            onClick={() => onChange(opt.value)}
            className={cn(
              "border-divider px-3 py-1.5 font-ui text-xs transition-colors [border-inline-end-width:1px] last:[border-inline-end-width:0]",
              value === opt.value ? "bg-p9 font-medium text-p1" : "bg-surface text-p9 hover:bg-p4",
            )}
          >
            {t(opt.labelKey)}
          </button>
        ))}
      </div>
      {selected && (
        <p className="mt-2 font-body text-xs leading-relaxed text-p8">{t(selected.descKey)}</p>
      )}
    </div>
  );
}

// ── Reset confirm dialog ──────────────────────────────────────────────────────

function ResetConfirmDialog({
  onConfirm,
  onCancel,
}: {
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("settings");
  return (
    <div
      className="bg-p10/30 fixed inset-0 z-50 flex items-center justify-center p-4"
      role="presentation"
    >
      <div
        className="w-full max-w-sm rounded-card bg-surface p-6 shadow-card"
        role="alertdialog"
        aria-modal="true"
        aria-label={t("privacy_reset_confirm_title")}
      >
        <p className="font-ui text-sm font-medium text-p10">{t("privacy_reset_confirm_title")}</p>
        <p className="mt-2 font-body text-sm text-p9">{t("privacy_reset_confirm_body")}</p>
        <div className="mt-5 flex justify-end gap-3">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-chip px-4 py-1.5 font-ui text-sm text-p8 hover:text-p9"
          >
            {t("privacy_reset_cancel")}
          </button>
          <button
            type="button"
            onClick={onConfirm}
            className="rounded-chip bg-p9 px-4 py-1.5 font-ui text-sm font-medium text-p1 hover:bg-p10"
          >
            {t("privacy_reset_confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export default function PrivacySettings() {
  const { t } = useTranslation("settings");
  const [confirmReset, setConfirmReset] = useState(false);

  const { data, isLoading } = usePrivacySettings();
  const setPolicies = useSetPrivacySettings();

  const trackerPolicy = data?.trackerPolicy ?? DEFAULT_TRACKER_POLICY;
  const remoteImagePolicy = data?.remoteImagePolicy ?? DEFAULT_REMOTE_IMAGE_POLICY;

  const apply = (next: { trackerPolicy?: TrackerPolicy; remoteImagePolicy?: ImagePolicy }) =>
    setPolicies.mutate({
      trackerPolicy: next.trackerPolicy ?? trackerPolicy,
      remoteImagePolicy: next.remoteImagePolicy ?? remoteImagePolicy,
    });

  return (
    <div className="max-w-xl space-y-4">
      {confirmReset && (
        <ResetConfirmDialog
          onConfirm={() => {
            setConfirmReset(false);
            setPolicies.mutate({
              trackerPolicy: DEFAULT_TRACKER_POLICY,
              remoteImagePolicy: DEFAULT_REMOTE_IMAGE_POLICY,
            });
          }}
          onCancel={() => setConfirmReset(false)}
        />
      )}

      <p className="section-label">{t("privacy_title")}</p>

      <SegmentedControl
        label={t("privacy_tracker_protection")}
        options={TRACKER_OPTIONS}
        value={trackerPolicy}
        onChange={(v) => apply({ trackerPolicy: v })}
        t={t}
      />

      <SegmentedControl
        label={t("privacy_image_loading")}
        options={IMAGE_OPTIONS}
        value={remoteImagePolicy}
        onChange={(v) => apply({ remoteImagePolicy: v })}
        t={t}
      />

      <div className="flex items-center justify-between pt-2">
        <p className="font-body text-xs text-p8">
          {isLoading ? t("privacy_loading") : t("privacy_immediate_note")}
        </p>
        <button
          type="button"
          onClick={() => setConfirmReset(true)}
          className="shrink-0 rounded-chip border border-divider bg-parchment px-3 py-1.5 font-ui text-xs text-p9 transition-colors hover:bg-p4"
        >
          {t("privacy_reset_defaults")}
        </button>
      </div>
    </div>
  );
}
