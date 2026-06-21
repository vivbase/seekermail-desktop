// Production i18n init (T008, 10 §1). react-i18next + ICU; English is the source
// of truth and the fallback. Missing keys fall back to `en`, never to a raw key.
//
// All locale resources are bundled eagerly via `import.meta.glob` over
// `resources/<locale>/<namespace>.json`. This keeps the app fully offline /
// local-first (no network backend needed) and means adding a locale is just
// dropping its `resources/<code>/` folder in — no edit here. The
// `i18next-http-backend` dependency remains available for a future lazy-loading
// optimization (10 §1, §3) but is intentionally unused while bundling is cheap.
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import ICU from "i18next-icu";

import { DEFAULT_LOCALE, SUPPORTED_LOCALES } from "./locales";

// One namespace per feature cluster keeps translation files small and lets the
// per-feature cards (T034–T049) own their strings without colliding (10 §1).
export const NAMESPACES = [
  "common",
  "nav",
  "errors",
  "search",
  "list",
  "reading",
  "compose",
  "settings",
  "agents",
  "aiProviders",
  "aiMatrix",
  "aiSetup",
  "aiDrafts",
  "audit",
  "legal",
  "team",
  "dashboard",
  "gte",
  "repository",
  "accountEmails",
  "activation",
] as const;

// Eagerly import every `resources/<locale>/<namespace>.json` and assemble the
// i18next resource tree: { [locale]: { [namespace]: bundle } }.
const modules = import.meta.glob("./resources/*/*.json", { eager: true });

const resources: Record<string, Record<string, Record<string, unknown>>> = {};
for (const [path, mod] of Object.entries(modules)) {
  const match = /\/resources\/([^/]+)\/([^/]+)\.json$/.exec(path);
  if (!match) continue;
  const locale = match[1];
  const namespace = match[2];
  if (!locale || !namespace) continue;
  const bundle = (mod as { default: Record<string, unknown> }).default;
  (resources[locale] ??= {})[namespace] = bundle;
}

void i18n
  .use(ICU)
  .use(initReactI18next)
  .init({
    lng: DEFAULT_LOCALE,
    fallbackLng: DEFAULT_LOCALE,
    supportedLngs: SUPPORTED_LOCALES,
    ns: NAMESPACES,
    defaultNS: "common",
    resources,
    interpolation: { escapeValue: false }, // React already escapes
    returnEmptyString: false, // empty translation → fall back, not blank
  });

export default i18n;
