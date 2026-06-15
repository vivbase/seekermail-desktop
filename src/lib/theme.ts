// Theme application (T050). The single place the `html.dark` class is managed.
// `applyTheme` is called (1) at boot from main.tsx with the persisted value and
// (2) from the Appearance page on every selection. "system" installs a
// prefers-color-scheme listener so macOS appearance changes apply live without
// touching the persisted preference.

export type ThemePreference = "light" | "dark" | "system";

export const THEME_SETTING_KEY = "ui.theme";

declare global {
  interface Window {
    /** Injected by the Rust setup hook before React mounts (FOUC guard). */
    __INITIAL_THEME__?: string;
  }
}

export function isThemePreference(value: unknown): value is ThemePreference {
  return value === "light" || value === "dark" || value === "system";
}

function media(): MediaQueryList | null {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return null;
  return window.matchMedia("(prefers-color-scheme: dark)");
}

export function systemPrefersDark(): boolean {
  return media()?.matches ?? false;
}

function setDarkClass(dark: boolean): void {
  document.documentElement.classList.toggle("dark", dark);
}

let systemListener: ((e: MediaQueryListEvent) => void) | null = null;

/** Apply a theme now; manages the system-appearance listener lifecycle. */
export function applyTheme(theme: ThemePreference): void {
  const mq = media();
  if (mq && systemListener) {
    mq.removeEventListener("change", systemListener);
    systemListener = null;
  }
  if (theme === "system") {
    setDarkClass(mq?.matches ?? false);
    if (mq) {
      systemListener = (e) => setDarkClass(e.matches);
      mq.addEventListener("change", systemListener);
    }
    return;
  }
  setDarkClass(theme === "dark");
}

/** The theme to paint before the IPC read resolves (boot path). */
export function initialThemeHint(): ThemePreference {
  const injected = typeof window !== "undefined" ? window.__INITIAL_THEME__ : undefined;
  return isThemePreference(injected) ? injected : "system";
}
