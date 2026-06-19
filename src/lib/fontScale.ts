// UI text-size scaling (analysis 25). The single place the global UI scale is
// managed. `applyFontScale` writes the `--ui-scale` CSS variable onto <html>; the
// `#root { zoom: var(--ui-scale) }` rule (styles/index.css) then scales the whole
// interface proportionally, so enlarging text can never break the layout.
//
// The preference persists as `app_settings.ui.font_scale`. The Rust setup hook
// injects `window.__INITIAL_FONT_SCALE__` before React mounts (FOUC guard),
// mirroring the theme mechanism in `lib/theme.ts`.

export const FONT_SCALE_SETTING_KEY = "ui.font_scale";

/** Clamp bounds — also the extremes offered on the Appearance page. */
export const FONT_SCALE_MIN = 0.9;
export const FONT_SCALE_MAX = 1.5;
export const DEFAULT_FONT_SCALE = 1;

/** Discrete steps offered to the user (Small → Largest). */
export const FONT_SCALE_STEPS = [0.9, 1, 1.15, 1.3, 1.5] as const;

declare global {
  interface Window {
    /** Injected by the Rust setup hook before React mounts (FOUC guard). */
    __INITIAL_FONT_SCALE__?: number;
  }
}

/** Coerce any input to a finite multiplier within [MIN, MAX]; default on junk.
 * Only genuine finite numbers are clamped — null/""/[] coerce to 0 under
 * `Number()`, so we reject non-numbers outright rather than clamp them to MIN. */
export function clampFontScale(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_FONT_SCALE;
  return Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, value));
}

/** Apply a scale now by setting the CSS variable the root element zooms by. */
export function applyFontScale(scale: number): void {
  document.documentElement.style.setProperty("--ui-scale", String(clampFontScale(scale)));
}

/** The scale to paint before the IPC read resolves (boot path). */
export function initialFontScaleHint(): number {
  const injected = typeof window !== "undefined" ? window.__INITIAL_FONT_SCALE__ : undefined;
  return clampFontScale(injected);
}

/** Index of the discrete step nearest to `current` (tolerates off-step values). */
function nearestStepIndex(current: number): number {
  let best = 0;
  let bestDist = Number.POSITIVE_INFINITY;
  FONT_SCALE_STEPS.forEach((step, i) => {
    const dist = Math.abs(step - current);
    if (dist < bestDist) {
      bestDist = dist;
      best = i;
    }
  });
  return best;
}

/** The next larger discrete step (clamped at the top) — for Cmd/Ctrl "+". */
export function nextFontScaleStep(current: number): number {
  const i = nearestStepIndex(current);
  return FONT_SCALE_STEPS[Math.min(FONT_SCALE_STEPS.length - 1, i + 1)] ?? DEFAULT_FONT_SCALE;
}

/** The next smaller discrete step (clamped at the bottom) — for Cmd/Ctrl "-". */
export function prevFontScaleStep(current: number): number {
  const i = nearestStepIndex(current);
  return FONT_SCALE_STEPS[Math.max(0, i - 1)] ?? DEFAULT_FONT_SCALE;
}
