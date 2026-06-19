// Appearance settings (T050). Theme persists to `app_settings.ui.theme` via the
// settings hooks and applies instantly through `applyTheme` (html.dark class).
// List density stays in the useUi store; language uses i18n.changeLanguage.
// Immediate-save throughout (F_H1 §4.3) — no Save/Cancel buttons.
import { useTranslation } from "react-i18next";

import {
  useFontScaleSetting,
  useReadingScaleSetting,
  useSetFontScale,
  useSetReadingScale,
  useSetTheme,
  useThemeSetting,
} from "@/ipc/queries/settings";
import { useUi, type Density } from "@/stores/ui";
import { cn } from "@/lib/cn";
import type { ThemePreference } from "@/lib/theme";
import { LOCALE_META } from "@/i18n/locales";

// ── Theme ─────────────────────────────────────────────────────────────────────

const THEME_OPTIONS: { value: ThemePreference; labelKey: string }[] = [
  { value: "system", labelKey: "appearance_theme_system" },
  { value: "light", labelKey: "appearance_theme_light" },
  { value: "dark", labelKey: "appearance_theme_dark" },
];

// ── Text size / UI scale (analysis 25) ──────────────────────────────────────────
// Each step is a whole-UI zoom multiplier; selecting one scales the entire
// interface proportionally so the layout never breaks. Numeric values mirror
// FONT_SCALE_STEPS in lib/fontScale.ts.

const TEXT_SIZE_OPTIONS: { value: number; labelKey: string }[] = [
  { value: 0.9, labelKey: "appearance_text_size_small" },
  { value: 1, labelKey: "appearance_text_size_default" },
  { value: 1.15, labelKey: "appearance_text_size_large" },
  { value: 1.3, labelKey: "appearance_text_size_larger" },
  { value: 1.5, labelKey: "appearance_text_size_largest" },
];

// Reading text size scales only the email body (analysis 25, Layer 2); the size
// words are shared with Text Size to keep one vocabulary.
const READING_SIZE_OPTIONS: { value: number; labelKey: string }[] = [
  { value: 0.9, labelKey: "appearance_text_size_small" },
  { value: 1, labelKey: "appearance_text_size_default" },
  { value: 1.15, labelKey: "appearance_text_size_large" },
  { value: 1.3, labelKey: "appearance_text_size_larger" },
  { value: 1.5, labelKey: "appearance_text_size_largest" },
];

// ── Language ──────────────────────────────────────────────────────────────────
// All supported locales come from the single source of truth in `@/i18n/locales`
// (LOCALE_META / SUPPORTED_LOCALES) so this picker stays in lockstep with the
// Dashboard switcher and i18next config. Native-script names are the sole
// sanctioned exception to the English-only UI rule.

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

  // Persisted UI scale (app_settings.ui.font_scale); 1 = 100% until first save.
  const { fontScale } = useFontScaleSetting();
  const setFontScale = useSetFontScale();

  // Persisted email reading scale (app_settings.ui.reading_font_scale).
  const { readingScale } = useReadingScaleSetting();
  const setReadingScale = useSetReadingScale();

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

      {/* Text size — whole-UI proportional scale (analysis 25) */}
      <SettingRow label={t("appearance_text_size")}>
        <div className="flex gap-2" role="group" aria-label={t("appearance_text_size")}>
          {TEXT_SIZE_OPTIONS.map((opt) => (
            <ToggleChip
              key={opt.value}
              active={Math.abs(fontScale - opt.value) < 0.001}
              label={t(opt.labelKey)}
              onClick={() => setFontScale.mutate(opt.value)}
            />
          ))}
        </div>
      </SettingRow>

      {/* Reading text size — scales the email body only (analysis 25, Layer 2) */}
      <SettingRow label={t("appearance_reading_size")}>
        <div className="flex gap-2" role="group" aria-label={t("appearance_reading_size")}>
          {READING_SIZE_OPTIONS.map((opt) => (
            <ToggleChip
              key={opt.value}
              active={Math.abs(readingScale - opt.value) < 0.001}
              label={t(opt.labelKey)}
              onClick={() => setReadingScale.mutate(opt.value)}
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
          {Object.entries(LOCALE_META).map(([code, meta]) => (
            <option key={code} value={code}>
              {meta.name}
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
