// Email-body reading text size (analysis 25, "Layer 2"). A second, independent
// multiplier that scales ONLY the sanitised email body — never the app chrome.
// `applyReadingScale` writes `--reading-scale` onto <html>; SanitizedMail's
// `.seeker-mail-body { font-size: calc(14px * var(--reading-scale)) }` is the only
// consumer, so this composes with the global UI scale without touching the rest of
// the interface. The preference persists as `app_settings.ui.reading_font_scale`.

export const READING_SCALE_SETTING_KEY = "ui.reading_font_scale";

/** Clamp bounds — also the extremes offered in the UI. */
export const READING_SCALE_MIN = 0.9;
export const READING_SCALE_MAX = 1.5;
export const DEFAULT_READING_SCALE = 1;

/** Discrete steps offered to the user (Small → Largest). */
export const READING_SCALE_STEPS = [0.9, 1, 1.15, 1.3, 1.5] as const;

/** Coerce any input to a finite multiplier within [MIN, MAX]; default on junk. */
export function clampReadingScale(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_READING_SCALE;
  return Math.min(READING_SCALE_MAX, Math.max(READING_SCALE_MIN, value));
}

/** Apply a scale now by setting the CSS variable the email body sizes against. */
export function applyReadingScale(scale: number): void {
  document.documentElement.style.setProperty("--reading-scale", String(clampReadingScale(scale)));
}

/** Index of the discrete step nearest to `current` (tolerates off-step values). */
function nearestStepIndex(current: number): number {
  let best = 0;
  let bestDist = Number.POSITIVE_INFINITY;
  READING_SCALE_STEPS.forEach((step, i) => {
    const dist = Math.abs(step - current);
    if (dist < bestDist) {
      bestDist = dist;
      best = i;
    }
  });
  return best;
}

/** The next larger discrete step (clamped at the top) — for the A+ button. */
export function nextReadingStep(current: number): number {
  const i = nearestStepIndex(current);
  return (
    READING_SCALE_STEPS[Math.min(READING_SCALE_STEPS.length - 1, i + 1)] ?? DEFAULT_READING_SCALE
  );
}

/** The next smaller discrete step (clamped at the bottom) — for the A− button. */
export function prevReadingStep(current: number): number {
  const i = nearestStepIndex(current);
  return READING_SCALE_STEPS[Math.max(0, i - 1)] ?? DEFAULT_READING_SCALE;
}

/** True when already at the smallest step (A− should be disabled). */
export function isMinReadingScale(scale: number): boolean {
  return clampReadingScale(scale) <= READING_SCALE_MIN;
}

/** True when already at the largest step (A+ should be disabled). */
export function isMaxReadingScale(scale: number): boolean {
  return clampReadingScale(scale) >= READING_SCALE_MAX;
}
