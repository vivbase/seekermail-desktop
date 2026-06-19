// UI-scale FOUC guard (analysis 25), sibling to theme-boot.ts. Loaded from
// index.html BEFORE the app bundle so `--ui-scale` is set before first paint. The
// Rust setup hook injects `window.__INITIAL_FONT_SCALE__` from
// `app_settings.ui.font_scale`; when absent (first run, dev browser) we fall back
// to 1.0 (no scaling). An external module (not an inline script) keeps the strict
// `default-src 'self'` CSP intact.
import { applyFontScale, initialFontScaleHint } from "@/lib/fontScale";

applyFontScale(initialFontScaleHint());
