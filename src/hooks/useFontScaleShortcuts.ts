// Global UI-scale keyboard shortcuts (analysis 25 follow-up). Cmd/Ctrl + "=" / "+"
// enlarges the whole interface a step, "-" / "_" shrinks it, and "0" resets to 100%.
// Registered once from AppShell so the app responds anywhere. Mirrors the browser's
// zoom keys; preventDefault stops the webview's own zoom so the persisted
// `ui.font_scale` setting stays the single source of truth.
import { useEffect, useRef } from "react";

import { useFontScaleSetting, useSetFontScale } from "@/ipc/queries/settings";
import { DEFAULT_FONT_SCALE, nextFontScaleStep, prevFontScaleStep } from "@/lib/fontScale";

export function useFontScaleShortcuts(): void {
  const { fontScale } = useFontScaleSetting();
  const setFontScale = useSetFontScale();

  // Keep the latest scale + mutate fn in refs so the listener is installed once
  // and never needs re-binding on each render.
  const scaleRef = useRef(fontScale);
  scaleRef.current = fontScale;
  const mutateRef = useRef(setFontScale.mutate);
  mutateRef.current = setFontScale.mutate;

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent): void {
      // Require the platform zoom modifier; ignore Alt-combos to avoid clashes.
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;

      let next: number | null = null;
      switch (e.key) {
        case "=":
        case "+":
          next = nextFontScaleStep(scaleRef.current);
          break;
        case "-":
        case "_":
          next = prevFontScaleStep(scaleRef.current);
          break;
        case "0":
          next = DEFAULT_FONT_SCALE;
          break;
        default:
          return;
      }

      e.preventDefault();
      if (next !== scaleRef.current) mutateRef.current(next);
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
