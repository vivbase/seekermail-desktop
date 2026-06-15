// Locale switch side effects (T008, 10 §5). Reproduces the prototype `setLang`
// behavior exactly: flip <html lang/dir>, swap the `script-*` class, then change
// the i18next language. RTL + font stacks follow from `dir`/`script` (scripts.css).
import i18n from "./index";
import { DEFAULT_LOCALE, LOCALE_META } from "./locales";

/** Apply a locale's direction/script to <html> and switch the language. */
export function applyLocale(code: string): void {
  const meta = LOCALE_META[code] ?? LOCALE_META[DEFAULT_LOCALE]!;
  const html = document.documentElement;
  html.lang = code;
  html.dir = meta.dir; // 'ltr' | 'rtl'
  html.className = html.className.replace(/\bscript-\S+/g, "").trim();
  html.classList.add(`script-${meta.script}`);
  void i18n.changeLanguage(code);
  persistLocale(code);
}

/**
 * Persist the chosen locale to `app_settings` (`ui.language`). The
 * `set_app_setting` command lands with Module H (v0.2+); until then this is a
 * no-op seam so the call site already matches 10 §5.
 */
function persistLocale(_code: string): void {
  // v0.2+: ipc('set_app_setting', { key: 'ui.language', value: JSON.stringify(_code) })
}
