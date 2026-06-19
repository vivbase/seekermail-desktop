// Unit tests for the email-body reading-scale helpers (analysis 25, Layer 2).
import { describe, it, expect, beforeEach } from "vitest";

import {
  applyReadingScale,
  clampReadingScale,
  DEFAULT_READING_SCALE,
  isMaxReadingScale,
  isMinReadingScale,
  nextReadingStep,
  prevReadingStep,
  READING_SCALE_MAX,
  READING_SCALE_MIN,
} from "./readingScale";

describe("clampReadingScale", () => {
  it("returns in-range values unchanged", () => {
    expect(clampReadingScale(1.15)).toBe(1.15);
  });

  it("clamps out-of-range values to the bounds", () => {
    expect(clampReadingScale(9)).toBe(READING_SCALE_MAX);
    expect(clampReadingScale(0.1)).toBe(READING_SCALE_MIN);
  });

  it("falls back to the default on non-numeric input", () => {
    expect(clampReadingScale(undefined)).toBe(DEFAULT_READING_SCALE);
    expect(clampReadingScale(null)).toBe(DEFAULT_READING_SCALE);
    expect(clampReadingScale("big")).toBe(DEFAULT_READING_SCALE);
  });
});

describe("applyReadingScale", () => {
  beforeEach(() => {
    document.documentElement.style.removeProperty("--reading-scale");
  });

  it("writes the clamped multiplier to the --reading-scale CSS variable", () => {
    applyReadingScale(1.3);
    expect(document.documentElement.style.getPropertyValue("--reading-scale")).toBe("1.3");
  });
});

describe("reading scale step helpers (A− / A+)", () => {
  it("steps up and down through the discrete steps", () => {
    expect(nextReadingStep(1)).toBe(1.15);
    expect(prevReadingStep(1)).toBe(0.9);
  });

  it("clamps at the ends and reports min/max", () => {
    expect(nextReadingStep(1.5)).toBe(1.5);
    expect(prevReadingStep(0.9)).toBe(0.9);
    expect(isMinReadingScale(0.9)).toBe(true);
    expect(isMaxReadingScale(1.5)).toBe(true);
    expect(isMinReadingScale(1)).toBe(false);
    expect(isMaxReadingScale(1)).toBe(false);
  });
});
