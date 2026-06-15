// FOUC guard (T050 §6). Loaded from index.html BEFORE the app bundle so the
// `html.dark` class is set before first paint. An external module (not an
// inline script) keeps the strict `default-src 'self'` CSP intact. The Rust
// setup hook injects `window.__INITIAL_THEME__` from `app_settings.ui.theme`;
// when absent (first run, dev browser) we follow the OS appearance.
import { applyTheme, initialThemeHint } from "@/lib/theme";

applyTheme(initialThemeHint());
