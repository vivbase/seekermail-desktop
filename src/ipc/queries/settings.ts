// TanStack Query hooks for the `app_settings` KV surface (T050/T051).
// Components consume these, never `ipc()` or `invoke` directly (07 §6).
// Values cross the wire as raw JSON strings; this layer owns (de)serialisation.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ImagePolicy, TrackerPolicy } from "@shared/bindings";

import {
  applyTheme,
  isThemePreference,
  THEME_SETTING_KEY,
  type ThemePreference,
} from "@/lib/theme";
import {
  applyFontScale,
  clampFontScale,
  DEFAULT_FONT_SCALE,
  FONT_SCALE_SETTING_KEY,
} from "@/lib/fontScale";
import {
  applyReadingScale,
  clampReadingScale,
  DEFAULT_READING_SCALE,
  READING_SCALE_SETTING_KEY,
} from "@/lib/readingScale";

import { ipc } from "../client";

export const settingKeys = {
  all: ["appSetting"] as const,
  detail: (key: string) => ["appSetting", key] as const,
};

/** `app_settings` keys owned by the privacy page (T051). */
export const TRACKER_POLICY_KEY = "privacy.tracker_policy";
export const REMOTE_IMAGE_POLICY_KEY = "privacy.remote_image_policy";

/** Built-in defaults — mirror the Rust `ensure_privacy_defaults` seed (T051 §6). */
export const DEFAULT_TRACKER_POLICY: TrackerPolicy = "block_known";
export const DEFAULT_REMOTE_IMAGE_POLICY: ImagePolicy = "block_all";

function parseJson<T>(raw: string | null): T | null {
  if (raw === null) return null;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

/** Read one settings key, JSON-decoded. `null` when unset or malformed. */
export function useAppSetting<T>(key: string) {
  return useQuery({
    queryKey: settingKeys.detail(key),
    queryFn: async () => parseJson<T>(await ipc("get_setting", { key })),
  });
}

/** Write one settings key (value is JSON-encoded here) and refresh its query. */
export function useSetAppSetting() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { key: string; value: unknown }) =>
      ipc("set_setting", { key: vars.key, value: JSON.stringify(vars.value) }),
    onSuccess: (_d, vars) => void qc.invalidateQueries({ queryKey: settingKeys.detail(vars.key) }),
  });
}

// ── E3 kill switch (T086) ─────────────────────────────────────────────────────

/**
 * `app_settings` key read by the E3 pipeline (T085): auto-send is paused while
 * the stored unix timestamp is in the future. Stored as a RAW integer string
 * (not JSON-wrapped) — the Rust side parses it directly.
 */
export const E3_PAUSED_UNTIL_KEY = "ai.e3_paused_until";

/** How long one kill-switch press pauses auto-send (F_E3 §5: 24 h). */
export const E3_PAUSE_SECS = 24 * 3600;

/** Tolerant decode: raw integer string preferred, JSON number accepted. */
function parseUnixSetting(raw: string | null): number {
  if (raw === null) return 0;
  const direct = Number(raw);
  if (Number.isFinite(direct)) return direct;
  try {
    const parsed: unknown = JSON.parse(raw);
    return typeof parsed === "number" && Number.isFinite(parsed) ? parsed : 0;
  } catch {
    return 0;
  }
}

/**
 * The pause deadline (unix seconds; 0 = not paused). Refetches once a minute
 * so the sidebar countdown and the audit banner stay roughly current.
 */
export function useE3PausedUntil() {
  return useQuery({
    queryKey: settingKeys.detail(E3_PAUSED_UNTIL_KEY),
    queryFn: async () => parseUnixSetting(await ipc("get_setting", { key: E3_PAUSED_UNTIL_KEY })),
    refetchInterval: 60_000,
  });
}

/** Write the pause deadline (pass 0 to resume early). Raw string on the wire. */
export function useSetE3PausedUntil() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (untilUnix: number) =>
      ipc("set_setting", { key: E3_PAUSED_UNTIL_KEY, value: String(untilUnix) }),
    onSuccess: () =>
      void qc.invalidateQueries({ queryKey: settingKeys.detail(E3_PAUSED_UNTIL_KEY) }),
  });
}

// ── Global AI master switch (T067, F_F5 §4.5) ─────────────────────────────────

/**
 * `app_settings` key honored by the F5 fallback router (commands::ai
 * ::set_ai_disabled): while the stored unix timestamp is in the future, EVERY
 * AI call is downgraded with reason `user_disabled`. Distinct from the E3 pause
 * above — that only demotes full-auto send; this disables all AI capabilities.
 */
export const AI_DISABLE_UNTIL_KEY = "ai.disable_until";

/** Preset disable windows (F_F5 §4.5: 24 h / 48 h / permanent). */
export const AI_DISABLE_24H_SECS = 24 * 3600;
export const AI_DISABLE_48H_SECS = 48 * 3600;

/**
 * "Permanent" is modeled as a far-future deadline (2100-01-01 UTC) rather than a
 * separate flag, so the existing timestamp reader needs no new shape. Any
 * deadline at or beyond {@link AI_DISABLE_PERMANENT_MIN} is shown as permanent
 * instead of a countdown.
 */
export const AI_DISABLE_PERMANENT_UNTIL = 4_102_444_800;
export const AI_DISABLE_PERMANENT_MIN = 4_000_000_000;

/** True when a deadline represents the "permanent" choice, not a timed window. */
export function isPermanentDisable(untilUnix: number): boolean {
  return untilUnix >= AI_DISABLE_PERMANENT_MIN;
}

/**
 * The global AI-disable deadline (unix seconds; 0 = AI active). Refetches once a
 * minute so the settings status and its countdown stay roughly current.
 */
export function useAiDisabledUntil() {
  return useQuery({
    queryKey: settingKeys.detail(AI_DISABLE_UNTIL_KEY),
    queryFn: async () => parseUnixSetting(await ipc("get_setting", { key: AI_DISABLE_UNTIL_KEY })),
    refetchInterval: 60_000,
  });
}

/**
 * Set or clear the global AI-disable deadline through the dedicated F5 command
 * (`set_ai_disabled`): pass a unix-seconds deadline to disable, or `null` to
 * restore AI immediately.
 */
export function useSetAiDisabled() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (untilUnix: number | null) => ipc("set_ai_disabled", { until: untilUnix }),
    onSuccess: () =>
      void qc.invalidateQueries({ queryKey: settingKeys.detail(AI_DISABLE_UNTIL_KEY) }),
  });
}

// ── Theme (T050) ──────────────────────────────────────────────────────────────

/** The persisted theme preference; defaults to "system" until set. */
export function useThemeSetting() {
  const query = useAppSetting<string>(THEME_SETTING_KEY);
  const theme: ThemePreference = isThemePreference(query.data) ? query.data : "system";
  return { ...query, theme };
}

/** Persist a theme choice and apply it to the document immediately. */
export function useSetTheme() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (theme: ThemePreference) => {
      // Apply before the IPC round-trip so the switch feels instant.
      applyTheme(theme);
      return ipc("set_global_pref", { key: THEME_SETTING_KEY, value: JSON.stringify(theme) });
    },
    onSuccess: () => void qc.invalidateQueries({ queryKey: settingKeys.detail(THEME_SETTING_KEY) }),
  });
}

// ── UI scale / text size (analysis 25) ─────────────────────────────────────────

/** The persisted UI scale multiplier; defaults to 1 (no scaling) until set. */
export function useFontScaleSetting() {
  const query = useAppSetting<number>(FONT_SCALE_SETTING_KEY);
  const fontScale =
    typeof query.data === "number" ? clampFontScale(query.data) : DEFAULT_FONT_SCALE;
  return { ...query, fontScale };
}

/** Persist a UI scale choice and apply it to the document immediately. */
export function useSetFontScale() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (scale: number) => {
      const clamped = clampFontScale(scale);
      // Apply before the IPC round-trip so the change feels instant.
      applyFontScale(clamped);
      return ipc("set_global_pref", {
        key: FONT_SCALE_SETTING_KEY,
        value: JSON.stringify(clamped),
      });
    },
    onSuccess: () =>
      void qc.invalidateQueries({ queryKey: settingKeys.detail(FONT_SCALE_SETTING_KEY) }),
  });
}

// ── Email reading text size (analysis 25, Layer 2) ──────────────────────────────

/** The persisted email-body reading scale; defaults to 1 until set. */
export function useReadingScaleSetting() {
  const query = useAppSetting<number>(READING_SCALE_SETTING_KEY);
  const readingScale =
    typeof query.data === "number" ? clampReadingScale(query.data) : DEFAULT_READING_SCALE;
  return { ...query, readingScale };
}

/** Persist a reading-scale choice and apply it to the document immediately. */
export function useSetReadingScale() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (scale: number) => {
      const clamped = clampReadingScale(scale);
      // Apply before the IPC round-trip so the change feels instant.
      applyReadingScale(clamped);
      return ipc("set_global_pref", {
        key: READING_SCALE_SETTING_KEY,
        value: JSON.stringify(clamped),
      });
    },
    onSuccess: () =>
      void qc.invalidateQueries({ queryKey: settingKeys.detail(READING_SCALE_SETTING_KEY) }),
  });
}

// ── Workbench double-click behavior (WB-21) ─────────────────────────────────────
// Whether double-clicking a surface (mail row, search result, …) opens it in a new TAB or a new
// WINDOW. Persisted under the ui.* namespace; default "tab".
export const WORKBENCH_DOUBLE_CLICK_KEY = "ui.workbench_double_click";
export type DoubleClickAction = "tab" | "window";

/** The persisted double-click action; defaults to "tab" until set. */
export function useDoubleClickActionSetting() {
  const query = useAppSetting<DoubleClickAction>(WORKBENCH_DOUBLE_CLICK_KEY);
  const action: DoubleClickAction = query.data === "window" ? "window" : "tab";
  return { ...query, action };
}

/** Persist the double-click action and refresh its query. */
export function useSetDoubleClickAction() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (action: DoubleClickAction) =>
      ipc("set_setting", {
        key: WORKBENCH_DOUBLE_CLICK_KEY,
        value: JSON.stringify(action),
      }),
    onSuccess: () =>
      void qc.invalidateQueries({ queryKey: settingKeys.detail(WORKBENCH_DOUBLE_CLICK_KEY) }),
  });
}

// ── Privacy (T051) ────────────────────────────────────────────────────────────

export interface PrivacySettings {
  trackerPolicy: TrackerPolicy;
  remoteImagePolicy: ImagePolicy;
}

/** Both privacy policies, falling back to the documented defaults. */
export function usePrivacySettings() {
  return useQuery({
    queryKey: ["privacySettings"],
    queryFn: async (): Promise<PrivacySettings> => {
      const [tracker, image] = await Promise.all([
        ipc("get_setting", { key: TRACKER_POLICY_KEY }),
        ipc("get_setting", { key: REMOTE_IMAGE_POLICY_KEY }),
      ]);
      return {
        trackerPolicy: parseJson<TrackerPolicy>(tracker) ?? DEFAULT_TRACKER_POLICY,
        remoteImagePolicy: parseJson<ImagePolicy>(image) ?? DEFAULT_REMOTE_IMAGE_POLICY,
      };
    },
  });
}

/** Persist both policies via `apply_privacy_policy` and refresh the cache. */
export function useSetPrivacySettings() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: PrivacySettings) =>
      ipc("apply_privacy_policy", {
        tracker_policy: vars.trackerPolicy,
        remote_image_policy: vars.remoteImagePolicy,
      }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["privacySettings"] });
      void qc.invalidateQueries({ queryKey: settingKeys.detail(TRACKER_POLICY_KEY) });
      void qc.invalidateQueries({ queryKey: settingKeys.detail(REMOTE_IMAGE_POLICY_KEY) });
    },
  });
}
