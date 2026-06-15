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

  it("falls back to en copy for an untranslated locale (no bare keys)", async () => {
    await i18n.changeLanguage("fr");
    // fr has no resources yet → must fall back to the English value, not the key.
    expect(i18n.t("nav_dashboard", { ns: "nav" })).toBe("Dashboard");
  });
});
