// Inline reading text-size stepper for the L2 reading view (analysis 25, Layer 2).
// A− / A+ adjust ONLY the email body via the persisted `ui.reading_font_scale`
// setting; the rest of the UI is unaffected. Small "A" shrinks, large "A" grows —
// the editorial display face makes the size difference legible at a glance.
import { useTranslation } from "react-i18next";

import { useReadingScaleSetting, useSetReadingScale } from "@/ipc/queries/settings";
import {
  isMaxReadingScale,
  isMinReadingScale,
  nextReadingStep,
  prevReadingStep,
} from "@/lib/readingScale";

const BTN_CLASS =
  "flex items-center justify-center rounded-chip px-2 py-1.5 font-display text-p8 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9 disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-p8";

export function ReadingSizeControl() {
  const { t } = useTranslation("reading");
  const { readingScale } = useReadingScaleSetting();
  const setReadingScale = useSetReadingScale();

  const atMin = isMinReadingScale(readingScale);
  const atMax = isMaxReadingScale(readingScale);

  return (
    <div className="flex items-center gap-0.5" role="group" aria-label={t("reading_size_label")}>
      <button
        type="button"
        onClick={() => setReadingScale.mutate(prevReadingStep(readingScale))}
        disabled={atMin}
        aria-label={t("reading_size_decrease")}
        title={t("reading_size_decrease")}
        className={BTN_CLASS}
      >
        <span className="text-xs leading-none">A</span>
      </button>
      <button
        type="button"
        onClick={() => setReadingScale.mutate(nextReadingStep(readingScale))}
        disabled={atMax}
        aria-label={t("reading_size_increase")}
        title={t("reading_size_increase")}
        className={BTN_CLASS}
      >
        <span className="text-base leading-none">A</span>
      </button>
    </div>
  );
}
