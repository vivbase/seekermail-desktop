// T093 — TEAM channel UI: message bubble layouts, the @ mention composer + send,
// and the channel container (seeded messages + member drawer). Off-Tauri, `ipc()`
// resolves from the stateful mock layer in client.ts.
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";
import type { Account } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import type { ImMessage } from "@/ipc/im";
import MessageBubble from "./MessageBubble";
import ChannelInput from "./ChannelInput";
import TeamChannel from "./TeamChannel";

const NOW = Math.floor(Date.now() / 1000);

const ACCOUNT: Account = {
  id: "demo-1",
  email: "you@example.com",
  displayName: "Work",
  provider: "imap",
  imapHost: null,
  imapPort: 993,
  smtpHost: null,
  smtpPort: 587,
  colorToken: "slate",
  badgeLabel: "W",
  roleType: "work",
  roleDescription: null,
  authLevel: 1,
  isPrimary: true,
  isActive: true,
  syncIntervalSecs: 300,
  lastSyncedAt: null,
  knowledgeDepthMonths: 12,
  createdAt: 0,
  updatedAt: 0,
};

function msg(over: Partial<ImMessage>): ImMessage {
  return {
    id: "m",
    channelId: "main",
    senderType: "system",
    senderId: "system",
    messageType: "text",
    content: JSON.stringify({ text: "hi" }),
    linkedEmailId: null,
    status: "resolved",
    createdAt: NOW,
    readAt: null,
    ...over,
  };
}

function withProviders(ui: ReactNode) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("MessageBubble", () => {
  it("renders a system message centered with no dark bubble", () => {
    const { container } = withProviders(
      <MessageBubble message={msg({ content: JSON.stringify({ text: "Sam joined." }) })} />,
    );
    expect(screen.getByText("Sam joined.")).toBeInTheDocument();
    expect(container.querySelector(".bg-p9")).toBeNull();
  });

  it("end-aligns a human message in a dark bubble", () => {
    const { container } = withProviders(
      <MessageBubble
        message={msg({
          id: "h",
          senderType: "human",
          senderId: "human",
          content: JSON.stringify({ text: "Thanks team" }),
        })}
      />,
    );
    expect(screen.getByText("Thanks team")).toBeInTheDocument();
    expect(container.querySelector(".ms-auto")).not.toBeNull();
  });

  it("shows the agent name chip for an agent message", () => {
    withProviders(
      <MessageBubble
        message={msg({
          id: "a",
          senderType: "agent",
          senderId: "demo-1",
          content: JSON.stringify({ text: "On it" }),
        })}
        account={ACCOUNT}
      />,
    );
    expect(screen.getByText("On it")).toBeInTheDocument();
    expect(screen.getByText("Work")).toBeInTheDocument();
  });

  it("surfaces the retrieval-state chip for a grounded agent reply (P-3)", () => {
    withProviders(
      <MessageBubble
        message={msg({
          id: "g",
          senderType: "agent",
          senderId: "demo-1",
          content: JSON.stringify({
            text: "Here is your summary.",
            retrieval: {
              semanticAvailable: true,
              semanticHits: 3,
              temporalHits: 0,
              aggregateFacts: 0,
              indexedMails: 2,
              totalMails: 5,
            },
          }),
        })}
        account={ACCOUNT}
      />,
    );
    expect(screen.getByText("Here is your summary.")).toBeInTheDocument();
    // A partial index reads as an honest "searched N of M", never a silent empty.
    expect(screen.getByText(/Searched 2 of 5 indexed emails/)).toBeInTheDocument();
  });

  it("shows no retrieval chip for a plain agent note", () => {
    withProviders(
      <MessageBubble
        message={msg({
          id: "p",
          senderType: "agent",
          senderId: "demo-1",
          content: JSON.stringify({ text: "No AI model is connected." }),
        })}
        account={ACCOUNT}
      />,
    );
    expect(screen.queryByText(/indexed emails/)).toBeNull();
    expect(screen.queryByText(/email used/)).toBeNull();
  });

  it("memory-grounded reply skips the coverage caution (P-4)", () => {
    withProviders(
      <MessageBubble
        message={msg({
          id: "mem",
          senderType: "agent",
          senderId: "demo-1",
          content: JSON.stringify({
            text: "Here is your inbox overview.",
            retrieval: {
              semanticAvailable: true,
              semanticHits: 0,
              temporalHits: 0,
              aggregateFacts: 3,
              memoryHits: 2,
              indexedMails: 1,
              totalMails: 5,
            },
          }),
        })}
        account={ACCOUNT}
      />,
    );
    // Memory/aggregate answers read the full store, so the partial-index caution
    // must not fire even though only 1 of 5 mails is embedded.
    expect(screen.queryByText(/Searched 1 of 5/)).toBeNull();
    // The two thread summaries count toward what the answer used.
    expect(screen.getByText(/2 emails used/)).toBeInTheDocument();
  });
});

describe("ChannelInput", () => {
  it("opens the @ mention picker and inserts the agent name", () => {
    withProviders(<ChannelInput accounts={[ACCOUNT]} />);
    const ta = screen.getByLabelText(/Message the team/) as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "@" } });
    const option = screen.getByRole("option", { name: /Work/ });
    fireEvent.click(option);
    expect(ta.value).toContain("@Work");
  });

  it("sends on Enter via post_im_message", async () => {
    const spy = vi.spyOn(client, "ipc");
    withProviders(<ChannelInput accounts={[ACCOUNT]} />);
    const ta = screen.getByLabelText(/Message the team/);
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    await waitFor(() => expect(spy.mock.calls.some((c) => c[0] === "post_im_message")).toBe(true));
  });
});

describe("TeamChannel", () => {
  it("renders seeded channel messages and toggles the member drawer", async () => {
    withProviders(<TeamChannel />);
    await waitFor(() => expect(screen.getByText(/Morning sync complete/)).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "Show members" }));
    expect(screen.getByRole("dialog", { name: "Members" })).toBeInTheDocument();
  });

  it("marks the channel read on open so the TEAM badge clears its unread half", async () => {
    // Seed one guaranteed-unread agent message so the effect fires regardless of
    // test order (the shared mock store is mutated by earlier mounts).
    await client.ipc("post_im_message", {
      channel_id: "main",
      sender_type: "agent",
      sender_id: "demo-1",
      message_type: "text",
      content: JSON.stringify({ text: "New thread flagged for you." }),
      linked_email_id: null,
    });
    const spy = vi.spyOn(client, "ipc");
    withProviders(<TeamChannel />);
    await waitFor(() =>
      expect(spy.mock.calls.some((c) => c[0] === "mark_im_channel_read")).toBe(true),
    );
  });
});
