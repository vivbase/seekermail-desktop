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
import { applyLocale } from "./i18n/applyLocale";
import { DEFAULT_LOCALE } from "./i18n/locales";
import { applyTheme, initialThemeHint, isThemePreference, THEME_SETTING_KEY } from "./lib/theme";

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

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("root element not found");

ReactDOM.createRoot(rootEl).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
