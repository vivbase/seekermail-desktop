// T081 — DraftCard render states (F_E6 §3.2): badge per status, expiry
// countdown, the data-type="draft" E2E selector, and keyboard affordances.
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import type { AiDraft } from "@shared/bindings";

import "@/i18n";
import { DraftCard, draftBadgeFor, hoursUntilExpiry } from "./DraftCard";

const NOW = Math.floor(Date.now() / 1000);

const BASE_DRAFT: AiDraft = {
  id: "ai-draft-1",
  triggerMailId: "m-1",
  accountId: "demo-1",
  toAddr: { name: "Alice Nguyen", email: "alice@northwind.co" },
  ccAddrs: [],
  subject: "Re: Q4 budget review — final numbers",
  bodyOriginal: "Hi Alice,\n\nConfirmed for the board deck.\n\nBest",
  bodyCurrent: "Hi Alice,\n\nConfirmed for the board deck.\n\nBest",
  isEdited: false,
  styleMatchScore: 0.92,
  triggerMode: "E2_semi",
  aiModel: "mock-model",
  knowledgeRefs: [],
  status: "pending",
  sendAfter: null,
  expiresAt: NOW + 72 * 3600,
  sentAt: null,
  discardedAt: null,
  discardReason: null,
  createdAt: NOW - 600,
  updatedAt: NOW - 600,
};

describe("DraftCard", () => {
  it("shows the DRAFT READY badge and expiry for a fresh pending draft", () => {
    render(<DraftCard draft={BASE_DRAFT} onOpen={vi.fn()} />);
    expect(screen.getByText("Draft Ready")).toBeInTheDocument();
    expect(screen.getByText("Expires in 72h")).toBeInTheDocument();
  });

  it("shows the EDITED badge for an edited draft", () => {
    render(
      <DraftCard draft={{ ...BASE_DRAFT, isEdited: true, status: "edited" }} onOpen={vi.fn()} />,
    );
    expect(screen.getByText("Edited")).toBeInTheDocument();
  });

  it("escalates to REVIEW NEEDED close to expiry", () => {
    render(<DraftCard draft={{ ...BASE_DRAFT, expiresAt: NOW + 3600 }} onOpen={vi.fn()} />);
    expect(screen.getByText("Review Needed")).toBeInTheDocument();
  });

  it("carries the data-type='draft' selector and keyboard affordances", () => {
    const onOpen = vi.fn();
    render(<DraftCard draft={BASE_DRAFT} onOpen={onOpen} />);
    const card = screen.getByRole("button", { name: /Q4 budget review/ });
    expect(card).toHaveAttribute("data-type", "draft");
    expect(card).toHaveAttribute("tabindex", "0");
    fireEvent.keyDown(card, { key: "Enter" });
    expect(onOpen).toHaveBeenCalledWith(BASE_DRAFT.id);
  });

  it("opens on click", () => {
    const onOpen = vi.fn();
    render(<DraftCard draft={BASE_DRAFT} onOpen={onOpen} />);
    fireEvent.click(screen.getByRole("button", { name: /Q4 budget review/ }));
    expect(onOpen).toHaveBeenCalledWith(BASE_DRAFT.id);
  });
});

describe("badge + expiry helpers", () => {
  it("maps statuses to badges", () => {
    expect(draftBadgeFor(BASE_DRAFT, NOW)).toBe("ready");
    expect(draftBadgeFor({ ...BASE_DRAFT, isEdited: true }, NOW)).toBe("edited");
    expect(draftBadgeFor({ ...BASE_DRAFT, expiresAt: NOW + 3600 }, NOW)).toBe("review");
  });

  it("computes whole hours until expiry", () => {
    expect(hoursUntilExpiry(NOW + 72 * 3600, NOW)).toBe(72);
    expect(hoursUntilExpiry(NOW - 10, NOW)).toBe(0);
    expect(hoursUntilExpiry(null, NOW)).toBeNull();
  });
});
