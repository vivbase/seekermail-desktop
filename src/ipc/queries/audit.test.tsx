// T086/T089 — mis-send threshold mutation + approved-draft-count derivation
// against a mocked `ipc`.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import type { AiDecisionRow } from "@shared/bindings";

import "@/i18n";
import * as client from "@/ipc/client";
import { useToastStore } from "@/components/ui/Toast";
import { misSendSettingKey, useApprovedDraftCount, useReportMisSend } from "./audit";

const NOW = Math.floor(Date.now() / 1000);

function wrapper({ children }: { children: React.ReactNode }) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

beforeEach(() => {
  useToastStore.setState({ toasts: [] });
});

afterEach(() => {
  vi.restoreAllMocks();
});

type IpcArgs = [string, Record<string, unknown> | undefined];

function mockIpcWithMisSendStore(existing: number[]) {
  const calls: IpcArgs[] = [];
  const spy = vi.spyOn(client, "ipc").mockImplementation(((cmd: string, args?: unknown) => {
    calls.push([cmd, args as Record<string, unknown> | undefined]);
    if (cmd === "get_setting") return Promise.resolve(JSON.stringify(existing));
    if (cmd === "set_setting") return Promise.resolve(null);
    if (cmd === "update_account_ai_settings") {
      return Promise.resolve({ accountId: "demo-1", authLevel: 2 });
    }
    return Promise.resolve(null);
  }) as typeof client.ipc);
  return { spy, calls };
}

describe("useReportMisSend (T086 trust downgrade)", () => {
  it("records the report without demoting below the threshold", async () => {
    const { calls } = mockIpcWithMisSendStore([NOW - 3600]); // 1 prior report
    const { result } = renderHook(() => useReportMisSend(), { wrapper });

    const demoted = await result.current.mutateAsync({ accountId: "demo-1" });

    expect(demoted).toBe(false);
    const setCall = calls.find(([cmd]) => cmd === "set_setting");
    expect(setCall?.[1]?.key).toBe(misSendSettingKey("demo-1"));
    const stored = JSON.parse(setCall?.[1]?.value as string) as number[];
    expect(stored).toHaveLength(2);
    expect(calls.some(([cmd]) => cmd === "update_account_ai_settings")).toBe(false);
  });

  it("demotes to Semi-Auto on the third report within 7 days", async () => {
    const { calls } = mockIpcWithMisSendStore([NOW - 3600, NOW - 7200]); // 2 prior
    const { result } = renderHook(() => useReportMisSend(), { wrapper });

    const demoted = await result.current.mutateAsync({ accountId: "demo-1" });

    expect(demoted).toBe(true);
    const updateCall = calls.find(([cmd]) => cmd === "update_account_ai_settings");
    expect(updateCall).toBeDefined();
    expect(updateCall?.[1]?.account_id).toBe("demo-1");
    expect((updateCall?.[1]?.params as { authLevel: number }).authLevel).toBe(2);
    await waitFor(() =>
      expect(useToastStore.getState().toasts.some((t) => t.message.includes("Semi-Auto"))).toBe(
        true,
      ),
    );
  });

  it("ignores reports older than the 7-day window", async () => {
    const { calls } = mockIpcWithMisSendStore([NOW - 8 * 86_400, NOW - 9 * 86_400]); // stale
    const { result } = renderHook(() => useReportMisSend(), { wrapper });

    const demoted = await result.current.mutateAsync({ accountId: "demo-1" });

    expect(demoted).toBe(false);
    const setCall = calls.find(([cmd]) => cmd === "set_setting");
    const stored = JSON.parse(setCall?.[1]?.value as string) as number[];
    expect(stored).toHaveLength(1); // only the fresh report survives
    expect(calls.some(([cmd]) => cmd === "update_account_ai_settings")).toBe(false);
  });
});

describe("useApprovedDraftCount (T086 unlock gate)", () => {
  it("counts only draft_sent decisions", async () => {
    const rows: Partial<AiDecisionRow>[] = [
      { id: "1", decisionType: "draft_sent" },
      { id: "2", decisionType: "draft_sent" },
      { id: "3", decisionType: "draft_created" },
    ];
    const spy = vi.spyOn(client, "ipc").mockResolvedValue(rows as AiDecisionRow[]);
    const { result } = renderHook(() => useApprovedDraftCount("demo-1"), { wrapper });

    await waitFor(() => expect(result.current.data).toBe(2));
    expect(spy).toHaveBeenCalledWith("list_ai_decisions", {
      params: {
        accountId: "demo-1",
        sinceUnix: 0,
        untilUnix: null,
        decisionTypes: ["draft_sent"],
        impact: null,
        limit: 1000,
        offset: null,
      },
    });
  });
});
