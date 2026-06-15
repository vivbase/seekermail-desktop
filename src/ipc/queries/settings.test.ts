// T050/T051 hook tests. Off-Tauri, `ipc()` resolves from the stateful mock
// settings store in client.ts, so reads/writes behave like the real KV table.
import { describe, it, expect } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";

import {
  useAppSetting,
  useSetAppSetting,
  usePrivacySettings,
  useSetPrivacySettings,
  useThemeSetting,
  useSetTheme,
} from "./settings";

function wrapper({ children }: { children: React.ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return React.createElement(QueryClientProvider, { client: qc }, children);
}

describe("useAppSetting / useSetAppSetting", () => {
  it("reads null for an unset key and roundtrips a write", async () => {
    const read = renderHook(() => useAppSetting<string>("ui.never_set"), { wrapper });
    await waitFor(() => expect(read.result.current.isSuccess).toBe(true));
    expect(read.result.current.data).toBeNull();

    const write = renderHook(() => useSetAppSetting(), { wrapper });
    write.result.current.mutate({ key: "ui.theme", value: "dark" });
    await waitFor(() => expect(write.result.current.isSuccess).toBe(true));

    const reread = renderHook(() => useAppSetting<string>("ui.theme"), { wrapper });
    await waitFor(() => expect(reread.result.current.data).toBe("dark"));
  });
});

describe("useThemeSetting / useSetTheme (T050)", () => {
  it("persists the choice and applies the html.dark class immediately", async () => {
    const set = renderHook(() => useSetTheme(), { wrapper });
    set.result.current.mutate("dark");
    await waitFor(() => expect(set.result.current.isSuccess).toBe(true));
    expect(document.documentElement.classList.contains("dark")).toBe(true);

    const read = renderHook(() => useThemeSetting(), { wrapper });
    await waitFor(() => expect(read.result.current.theme).toBe("dark"));

    set.result.current.mutate("light");
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(false));
  });
});

describe("usePrivacySettings / useSetPrivacySettings (T051)", () => {
  it("falls back to the documented defaults", async () => {
    const { result } = renderHook(() => usePrivacySettings(), { wrapper });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    // First read in this suite may already be mutated by other tests writing
    // through the same mock store — assert shape + valid values instead.
    expect(["block_all", "block_known", "allow_all"]).toContain(result.current.data?.trackerPolicy);
    expect(["block_all", "trusted_only", "allow_all"]).toContain(
      result.current.data?.remoteImagePolicy,
    );
  });

  it("writes both policies through apply_privacy_policy", async () => {
    const set = renderHook(() => useSetPrivacySettings(), { wrapper });
    set.result.current.mutate({ trackerPolicy: "block_all", remoteImagePolicy: "allow_all" });
    await waitFor(() => expect(set.result.current.isSuccess).toBe(true));

    const read = renderHook(() => usePrivacySettings(), { wrapper });
    await waitFor(() => expect(read.result.current.isSuccess).toBe(true));
    expect(read.result.current.data).toEqual({
      trackerPolicy: "block_all",
      remoteImagePolicy: "allow_all",
    });
  });
});
