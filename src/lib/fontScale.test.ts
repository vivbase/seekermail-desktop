// Unit tests for the UI-scale helpers (analysis 25).
import { describe, it, expect, beforeEach, afterEach } from "vitest";

import {
  applyFontScale,
  clampFontScale,
  DEFAULT_FONT_SCALE,
  FONT_SCALE_MAX,
  FONT_SCALE_MIN,
  initialFontScaleHint,
  nextFontScaleStep,
  prevFontScaleStep,
} from "./fontScale";

describe("clampFontScale", () => {
  it("returns in-range values unchanged", () => {
    expect(clampFontScale(1)).toBe(1);
    expect(clampFontScale(1.15)).toBe(1.15);
  });

  it("clamps out-of-range values to the bounds", () => {
    expect(clampFontScale(5)).toBe(FONT_SCALE_MAX);
    expect(clampFontScale(0.1)).toBe(FONT_SCALE_MIN);
  });

  it("falls back to the default on non-numeric input", () => {
    expect(clampFontScale(undefined)).toBe(DEFAULT_FONT_SCALE);
    expect(clampFontScale(Number.NaN)).toBe(DEFAULT_FONT_SCALE);
    expect(clampFontScale("big")).toBe(DEFAULT_FONT_SCALE);
    expect(clampFontScale(null)).toBe(DEFAULT_FONT_SCALE);
  });
});

describe("applyFontScale", () => {
  beforeEach(() => {
    document.documentElement.style.removeProperty("--ui-scale");
  });

  it("writes the clamped multiplier to the --ui-scale CSS variable", () => {
    applyFontScale(1.3);
    expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe("1.3");
  });

  it("clamps before writing", () => {
    applyFontScale(99);
    expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe(
      String(FONT_SCALE_MAX),
    );
  });
});

describe("initialFontScaleHint", () => {
  afterEach(() => {
    delete window.__INITIAL_FONT_SCALE__;
  });

  it("uses the injected global when present", () => {
    window.__INITIAL_FONT_SCALE__ = 1.15;
    expect(initialFontScaleHint()).toBe(1.15);
  });

  it("clamps an out-of-range injected global", () => {
    window.__INITIAL_FONT_SCALE__ = 10;
    expect(initialFontScaleHint()).toBe(FONT_SCALE_MAX);
  });

  it("falls back to the default when the global is absent", () => {
    delete window.__INITIAL_FONT_SCALE__;
    expect(initialFontScaleHint()).toBe(DEFAULT_FONT_SCALE);
  });
});

describe("font scale step helpers (Cmd +/-)", () => {
  it("steps up and down through the discrete steps", () => {
    expect(nextFontScaleStep(1)).toBe(1.15);
    expect(prevFontScaleStep(1)).toBe(0.9);
  });

  it("clamps at the ends", () => {
    expect(nextFontScaleStep(1.5)).toBe(1.5);
    expect(prevFontScaleStep(0.9)).toBe(0.9);
  });

  it("snaps an off-step value to the nearest step's neighbour", () => {
    // 1.2 is nearest to 1.15: next → 1.3, prev → 1.
    expect(nextFontScaleStep(1.2)).toBe(1.3);
    expect(prevFontScaleStep(1.2)).toBe(1);
  });
});
