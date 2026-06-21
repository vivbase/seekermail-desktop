import { describe, it, expect, afterEach } from "vitest";

import i18n from "./index";
import { applyLocale } from "./applyLocale";

afterEach(() => {
  applyLocale("en");
});

describe("applyLocale()", () => {
  it("switches to RTL + arabic script for ar", () => {
    applyLocale("ar");
    expect(document.documentElement.dir).toBe("rtl");
    expect(document.documentElement.lang).toBe("ar");
    expect(document.documentElement.className).toContain("script-arabic");
  });

  it("uses the cjk script stack for ja", () => {
    applyLocale("ja");
    expect(document.documentElement.dir).toBe("ltr");
    expect(document.documentElement.className).toContain("script-cjk");
    expect(document.documentElement.className).not.toContain("script-arabic");
  });

  it("resets to ltr + latin for en", () => {
    applyLocale("ar");
    applyLocale("en");
    expect(document.documentElement.dir).toBe("ltr");
    expect(document.documentElement.className).toContain("script-latin");
  });

  it("serves localized copy for a translated locale (fr), not a bare key", async () => {
    await i18n.changeLanguage("fr");
    // fr is now fully translated → it must return localized copy, never the bare
    // key, and never the English string.
    const value = i18n.t("nav_dashboard", { ns: "nav" });
    expect(value).not.toBe("nav_dashboard");
    expect(value).not.toBe("Dashboard");
  });

  it("falls back to en for a key missing in the active locale (no bare keys)", async () => {
    // A key present only in `en` must resolve to the English copy under any other
    // locale, never surface as a raw key.
    i18n.addResourceBundle("en", "nav", { __probe_en_only__: "Probe EN" }, true, true);
    await i18n.changeLanguage("ja");
    expect(i18n.t("__probe_en_only__", { ns: "nav" })).toBe("Probe EN");
  });
});
