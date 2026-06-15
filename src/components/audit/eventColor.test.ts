// T089 — EVENT_COLOR token map: known types map to their tokens, unknown
// types fall back to the neutral var(--p9).
import { describe, it, expect } from "vitest";

import { EVENT_COLOR, eventColorVar, eventTypeLabel, supportsMisSendReport } from "./eventColor";

describe("eventColorVar", () => {
  it("maps the documented decision types to their tokens", () => {
    expect(eventColorVar("draft_sent")).toBe("var(--green)");
    expect(eventColorVar("e3_auto_sent")).toBe("var(--green)");
    expect(eventColorVar("draft_discarded")).toBe("var(--p7)");
    expect(eventColorVar("draft_expired")).toBe("var(--p7)");
    expect(eventColorVar("risk_intercepted")).toBe("var(--terra)");
    expect(eventColorVar("e4_sensitive")).toBe("var(--terra)");
    expect(eventColorVar("trust_downgraded")).toBe("var(--amber)");
  });

  it("falls back to var(--p9) for unknown types", () => {
    expect(eventColorVar("some_future_event")).toBe("var(--p9)");
    expect(eventColorVar("")).toBe("var(--p9)");
  });

  it("only ever yields CSS variable tokens — no raw hex", () => {
    for (const value of Object.values(EVENT_COLOR)) {
      expect(value).toMatch(/^var\(--[a-z0-9]+\)$/);
    }
  });
});

describe("eventTypeLabel", () => {
  it("humanizes slugs", () => {
    expect(eventTypeLabel("draft_sent")).toBe("Draft Sent");
    expect(eventTypeLabel("e3_auto_sent")).toBe("E3 Auto Sent");
  });
});

describe("supportsMisSendReport", () => {
  it("allows feedback only on sent events", () => {
    expect(supportsMisSendReport("draft_sent")).toBe(true);
    expect(supportsMisSendReport("e3_auto_sent")).toBe(true);
    expect(supportsMisSendReport("auto_reply_sent")).toBe(true);
    expect(supportsMisSendReport("draft_created")).toBe(false);
    expect(supportsMisSendReport("risk_intercepted")).toBe(false);
  });
});
