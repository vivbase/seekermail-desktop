import { beforeEach, describe, expect, it } from "vitest";

import type { MailDetail } from "@shared/bindings";

import { useAiReplyCard } from "./aiReplyCard";

const mail = { id: "m1" } as unknown as MailDetail;

describe("aiReplyCard store (E1 inline draft card)", () => {
  beforeEach(() => useAiReplyCard.getState().close());

  it("opens for a mail + scope", () => {
    useAiReplyCard.getState().openCard(mail, "reply-all");
    const s = useAiReplyCard.getState();
    expect(s.open).toBe(true);
    expect(s.minimized).toBe(false);
    expect(s.mail?.id).toBe("m1");
    expect(s.scope).toBe("reply-all");
  });

  it("minimises to the chip without losing the draft context, then resumes", () => {
    useAiReplyCard.getState().openCard(mail, "reply");
    useAiReplyCard.getState().minimize();
    let s = useAiReplyCard.getState();
    expect(s.minimized).toBe(true);
    expect(s.open).toBe(true); // still mounted
    expect(s.mail?.id).toBe("m1"); // context kept

    useAiReplyCard.getState().resume();
    s = useAiReplyCard.getState();
    expect(s.minimized).toBe(false);
    expect(s.open).toBe(true);
  });

  it("close clears the card and its mail", () => {
    useAiReplyCard.getState().openCard(mail, "reply");
    useAiReplyCard.getState().close();
    const s = useAiReplyCard.getState();
    expect(s.open).toBe(false);
    expect(s.minimized).toBe(false);
    expect(s.mail).toBeNull();
  });
});
