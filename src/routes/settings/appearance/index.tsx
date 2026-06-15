// Appearance settings (T050). Theme persists to `app_settings.ui.theme` via the
// settings hooks and applies instantly through `applyTheme` (html.dark class).
// List density stays in the useUi store; language uses i18n.changeLanguage.
// Immediate-save throughout (F_H1 §4.3) — no Save/Cancel buttons.
import { useTranslation } from "react-i18next";

import { useSetTheme, useThemeSetting } from "@/ipc/queries/settings";
import { useUi, type Density } from "@/stores/ui";
import { cn } from "@/lib/cn";
import type { ThemePreference } from "@/lib/theme";

// ── Theme ─────────────────────────────────────────────────────────────────────

const THEME_OPTIONS: { value: ThemePreference; labelKey: string }[] = [
  { value: "system", labelKey: "appearance_theme_system" },
  { value: "light", labelKey: "appearance_theme_light" },
  { value: "dark", labelKey: "appearance_theme_dark" },
];

// ── Language ──────────────────────────────────────────────────────────────────

interface LangOption {
  code: string;
  /** Native-script label — the sole exception to the "English UI only" rule. */
  nativeLabel: string;
}

const LANG_OPTIONS: LangOption[] = [
  { code: "en", nativeLabel: "English" },
  { code: "zh-CN", nativeLabel: "简体中文" },
  { code: "ja", nativeLabel: "日本語" },
  { code: "es", nativeLabel: "Español" },
];

// ── Density ───────────────────────────────────────────────────────────────────

const DENSITY_OPTIONS: { value: Density; labelKey: string }[] = [
  { value: "comfortable", labelKey: "appearance_density_comfortable" },
  { value: "compact", labelKey: "appearance_density_compact" },
];

// ── Component ─────────────────────────────────────────────────────────────────

export default function AppearanceSettings() {
  const { t, i18n } = useTranslation("settings");

  // Persisted preference (app_settings.ui.theme); "system" until first save.
  const { theme } = useThemeSetting();
  const setTheme = useSetTheme();

  const density = useUi((s) => s.density);
  const setDensity = useUi((s) => s.setDensity);

  const currentLang = i18n.language;

  return (
    <div className="max-w-xl space-y-8">
      <p className="section-label">{t("section_general")}</p>

      {/* Theme */}
      <SettingRow label={t("appearance_theme")}>
        <div className="flex gap-2" role="group" aria-label={t("appearance_theme")}>
          {THEME_OPTIONS.map((opt) => (
            <ToggleChip
              key={opt.value}
              active={theme === opt.value}
              label={t(opt.labelKey)}
              onClick={() => setTheme.mutate(opt.value)}
            />
          ))}
        </div>
      </SettingRow>

      {/* Density */}
      <SettingRow label={t("appearance_density")}>
        <div className="flex gap-2" role="group" aria-label={t("appearance_density")}>
          {DENSITY_OPTIONS.map((opt) => (
            <ToggleChip
              key={opt.value}
              active={density === opt.value}
              label={t(opt.labelKey)}
              onClick={() => setDensity(opt.value)}
            />
          ))}
        </div>
      </SettingRow>

      {/* Language */}
      <SettingRow label={t("appearance_language")}>
        <select
          value={currentLang}
          onChange={(e) => void i18n.changeLanguage(e.target.value)}
          aria-label={t("appearance_language")}
          className="rounded-chip border border-divider bg-surface px-3 py-1.5 font-ui text-sm text-p9 focus:outline focus:outline-2 focus:outline-p9"
        >
          {LANG_OPTIONS.map((lang) => (
            <option key={lang.code} value={lang.code}>
              {lang.nativeLabel}
            </option>
          ))}
        </select>
      </SettingRow>
    </div>
  );
}

// ── Sub-components ────────────────────────────────────────────────────────────

function SettingRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-2">
      <p className="font-ui text-xs font-medium uppercase tracking-wider text-p8">{label}</p>
      {children}
    </div>
  );
}

function ToggleChip({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "rounded-chip border px-3 py-1.5 font-ui text-sm transition-colors",
        active
          ? "border-transparent bg-p9 text-p1"
          : "border-divider bg-surface text-p9 hover:bg-p4",
      )}
    >
      {label}
    </button>
  );
}
