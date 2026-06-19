// senderAvatar — the L0 mail-card avatar must identify the sender, not the account
// (the old code reused the account badge, so every row showed the same letter/color).
import { describe, it, expect } from "vitest";

import { senderColorToken, senderInitial } from "./senderAvatar";

const PANEL_PALETTE = ["terra", "slate", "sage", "amber"];

describe("senderInitial", () => {
  it("uses the first letter of the email address, upper-cased", () => {
    expect(senderInitial("agentboyisme@gmail.com")).toBe("A");
    expect(senderInitial("zoe@example.org", "Some Name")).toBe("Z");
  });

  it("skips leading non-alphanumeric characters", () => {
    expect(senderInitial("  .hidden@x.io")).toBe("H");
  });

  it("falls back to the display name only when the email is blank", () => {
    expect(senderInitial("", "Google")).toBe("G");
    expect(senderInitial(null, "naomi")).toBe("N");
  });

  it("returns a neutral placeholder when nothing is usable", () => {
    expect(senderInitial("", "")).toBe("?");
    expect(senderInitial(null, null)).toBe("?");
  });
});

describe("senderColorToken", () => {
  it("always resolves to a dashboard panel token", () => {
    for (const email of ["a@b.com", "google@x.io", "q@z.dev", "team@seeker.app"]) {
      expect(PANEL_PALETTE).toContain(senderColorToken(email));
    }
  });

  it("is deterministic and normalizes case + whitespace", () => {
    expect(senderColorToken("Sam@Work.com")).toBe(senderColorToken("  sam@work.com  "));
  });

  it("separates senders across more than one color", () => {
    const tokens = new Set(
      ["alex@a.com", "blair@b.com", "casey@c.com", "drew@d.com", "evan@e.com", "fran@f.com"].map(
        senderColorToken,
      ),
    );
    expect(tokens.size).toBeGreaterThan(1);
  });
});
