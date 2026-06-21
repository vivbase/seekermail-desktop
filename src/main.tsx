// React root (07 §2). Mounts providers above the router: TanStack Query for server
// state, react-i18next for copy. Design tokens + i18n init load here once.
import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "./styles/index.css";
import "./styles/fonts"; // bundled Playfair/Lora/DM Sans/DM Mono (local, no CDN)
import "./styles/prototype.css"; // 1:1 ported prototype stylesheet (tokens + component classes)
import "./i18n"; // initialize i18next (English default locale)
import App from "./App";
import { ipc, registerIpcEvents } from "./ipc";
import { installExternalLinkHandler } from "./lib/externalLinks";
import { applyLocale } from "./i18n/applyLocale";
import { DEFAULT_LOCALE } from "./i18n/locales";
import { applyTheme, initialThemeHint, isThemePreference, THEME_SETTING_KEY } from "./lib/theme";
import {
  applyFontScale,
  clampFontScale,
  FONT_SCALE_SETTING_KEY,
  initialFontScaleHint,
} from "./lib/fontScale";
import {
  applyReadingScale,
  clampReadingScale,
  READING_SCALE_SETTING_KEY,
} from "./lib/readingScale";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
      staleTime: 5_000,
    },
  },
});

// Register backend event → query-invalidation listeners once (no-op off-Tauri).
registerIpcEvents(queryClient);

// Route external link clicks (in mail HTML, drafts, repository) to the OS
// default browser instead of letting them navigate the app's own webview.
installExternalLinkHandler();

// Apply the persisted/default locale: sets <html lang/dir/script-*>.
applyLocale(DEFAULT_LOCALE);

// Theme (T050): theme-boot.ts already painted the injected/system hint before
// this bundle ran; re-assert it (HMR safety), then reconcile against the
// persisted `ui.theme` setting in case the injected global was missing.
applyTheme(initialThemeHint());
void ipc("get_setting", { key: THEME_SETTING_KEY })
  .then((raw) => {
    const parsed: unknown = raw === null ? null : JSON.parse(raw);
    if (isThemePreference(parsed) && parsed !== initialThemeHint()) applyTheme(parsed);
  })
  .catch(() => {
    // Backend not ready / first run — the boot hint stays in effect.
  });

// UI scale (analysis 25): same pattern as theme — font-scale-boot.ts already set
// --ui-scale from the injected hint; re-assert it (HMR safety), then reconcile
// against the persisted `ui.font_scale` setting in case the global was missing.
applyFontScale(initialFontScaleHint());
void ipc("get_setting", { key: FONT_SCALE_SETTING_KEY })
  .then((raw) => {
    const parsed: unknown = raw === null ? null : JSON.parse(raw);
    if (typeof parsed === "number") {
      const next = clampFontScale(parsed);
      if (next !== initialFontScaleHint()) applyFontScale(next);
    }
  })
  .catch(() => {
    // Backend not ready / first run — the boot hint stays in effect.
  });

// Reading text size (analysis 25, Layer 2): no boot script (the email body is not
// visible at first paint), but apply the persisted value so an opened mail renders
// at the chosen size immediately.
void ipc("get_setting", { key: READING_SCALE_SETTING_KEY })
  .then((raw) => {
    const parsed: unknown = raw === null ? null : JSON.parse(raw);
    if (typeof parsed === "number") applyReadingScale(clampReadingScale(parsed));
  })
  .catch(() => {
    // Backend not ready / first run — the body stays at 100%.
  });

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("root element not found");

ReactDOM.createRoot(rootEl).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
