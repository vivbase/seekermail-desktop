import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";

vi.mock("@/ipc/client", () => ({ ipc: vi.fn(() => Promise.resolve("w1")) }));

import { ipc } from "@/ipc/client";
import { useOpenInNewWindow } from "./useOpenInNewWindow";
import { decodeBootToken } from "@/lib/bootToken";

describe("useOpenInNewWindow (T2)", () => {
  it("spawns a window with an encoded boot token", () => {
    const { result } = renderHook(() => useOpenInNewWindow());
    result.current({ route: { page: "thread", params: { mailId: "5" } }, accountId: "x" });

    expect(ipc).toHaveBeenCalledTimes(1);
    const call = vi.mocked(ipc).mock.calls[0]!;
    expect(call[0]).toBe("workbench_open_window");
    const args = call[1] as { boot: string; at: null };
    expect(decodeBootToken(args.boot)).toEqual({
      route: { page: "thread", params: { mailId: "5" } },
      accountId: "x",
    });
  });
});
