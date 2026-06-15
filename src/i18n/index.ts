// Production i18n init (T008, 10 §1). react-i18next + ICU; English is the source
// of truth and the fallback. Missing keys fall back to `en`, never to a raw key.
//
// v0.1 bundles the `en` resources inline (no server needed for dev/tests). The
// `i18next-http-backend` dependency is installed for the lazy per-locale loading
// that other locales will use as translations land (10 §1, §3).
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import ICU from "i18next-icu";

import { DEFAULT_LOCALE, SUPPORTED_LOCALES } from "./locales";
import enCommon from "./resources/en/common.json";
import enNav from "./resources/en/nav.json";
import enErrors from "./resources/en/errors.json";
import enSearch from "./resources/en/search.json";
import enList from "./resources/en/list.json";
import enReading from "./resources/en/reading.json";
import enCompose from "./resources/en/compose.json";
import enSettings from "./resources/en/settings.json";
import enAgents from "./resources/en/agents.json";
import enAiProviders from "./resources/en/aiProviders.json";
import enAiMatrix from "./resources/en/aiMatrix.json";
import enAiSetup from "./resources/en/aiSetup.json";
import enAiDrafts from "./resources/en/aiDrafts.json";
import enAudit from "./resources/en/audit.json";
import enLegal from "./resources/en/legal.json";
import enTeam from "./resources/en/team.json";
import enDashboard from "./resources/en/dashboard.json";
import enGte from "./resources/en/gte.json";
import enRepository from "./resources/en/repository.json";
import enAccountEmails from "./resources/en/accountEmails.json";

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
] as const;

void i18n
  .use(ICU)
  .use(initReactI18next)
  .init({
    lng: DEFAULT_LOCALE,
    fallbackLng: DEFAULT_LOCALE,
    supportedLngs: SUPPORTED_LOCALES,
    ns: NAMESPACES,
    defaultNS: "common",
    resources: {
      en: {
        common: enCommon,
        nav: enNav,
        errors: enErrors,
        search: enSearch,
        list: enList,
        reading: enReading,
        compose: enCompose,
        settings: enSettings,
        agents: enAgents,
        aiProviders: enAiProviders,
        aiMatrix: enAiMatrix,
        aiSetup: enAiSetup,
        aiDrafts: enAiDrafts,
        audit: enAudit,
        legal: enLegal,
        team: enTeam,
        dashboard: enDashboard,
        gte: enGte,
        repository: enRepository,
        accountEmails: enAccountEmails,
      },
    },
    interpolation: { escapeValue: false }, // React already escapes
    returnEmptyString: false, // empty translation → fall back, not blank
  });

export default i18n;
