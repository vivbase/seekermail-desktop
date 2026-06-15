// The 21 required locales and their metadata (T008, 10 §2, root CLAUDE.md).
// `dir` drives RTL; `script` selects the font stack (scripts.css). The production
// analogue of the prototype's `LANG_META`.

export type Direction = "ltr" | "rtl";
export type ScriptClass =
  | "latin"
  | "cjk"
  | "arabic"
  | "hebrew"
  | "devanagari"
  | "bengali"
  | "cyrillic";

export interface LocaleMeta {
  /** Short chip label for the switcher. */
  label: string;
  /** Language name in its OWN script (the sole sanctioned non-English UI text). */
  name: string;
  dir: Direction;
  script: ScriptClass;
}

export const LOCALE_META: Record<string, LocaleMeta> = {
  en: { label: "EN", name: "English", dir: "ltr", script: "latin" },
  "zh-CN": { label: "中", name: "简体中文", dir: "ltr", script: "cjk" },
  "zh-TW": { label: "繁", name: "繁體中文", dir: "ltr", script: "cjk" },
  ja: { label: "日", name: "日本語", dir: "ltr", script: "cjk" },
  ko: { label: "한", name: "한국어", dir: "ltr", script: "cjk" },
  vi: { label: "VI", name: "Tiếng Việt", dir: "ltr", script: "latin" },
  ar: { label: "ع", name: "العربية", dir: "rtl", script: "arabic" },
  es: { label: "ES", name: "Español", dir: "ltr", script: "latin" },
  pt: { label: "PT", name: "Português", dir: "ltr", script: "latin" },
  fr: { label: "FR", name: "Français", dir: "ltr", script: "latin" },
  de: { label: "DE", name: "Deutsch", dir: "ltr", script: "latin" },
  he: { label: "ע", name: "עברית", dir: "rtl", script: "hebrew" },
  hi: { label: "हि", name: "हिन्दी", dir: "ltr", script: "devanagari" },
  bn: { label: "বা", name: "বাংলা", dir: "ltr", script: "bengali" },
  ur: { label: "اُ", name: "اردو", dir: "rtl", script: "arabic" },
  it: { label: "IT", name: "Italiano", dir: "ltr", script: "latin" },
  nl: { label: "NL", name: "Nederlands", dir: "ltr", script: "latin" },
  pl: { label: "PL", name: "Polski", dir: "ltr", script: "latin" },
  ru: { label: "RU", name: "Русский", dir: "ltr", script: "cyrillic" },
  tr: { label: "TR", name: "Türkçe", dir: "ltr", script: "latin" },
  sv: { label: "SV", name: "Svenska", dir: "ltr", script: "latin" },
};

/** The ordered code list passed to i18next `supportedLngs`. */
export const SUPPORTED_LOCALES = Object.keys(LOCALE_META);

/** Locales that render right-to-left. */
export const RTL_LOCALES = SUPPORTED_LOCALES.filter((c) => LOCALE_META[c]?.dir === "rtl");

export const DEFAULT_LOCALE = "en";
