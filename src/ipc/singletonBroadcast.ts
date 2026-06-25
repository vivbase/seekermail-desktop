// Cross-window singleton routing (WB-17, Model S). Agent-IM / Dashboard / Pending are global
// singletons — only one window should show each. Windows coordinate over a frontend broadcast
// (`@tauri-apps/api` event emit/listen, allowed inside src/ipc/) with no backend involvement.
//
// Flow: an opener BROADCASTS a query {page, nonce} then waits briefly. Any window already showing
// that page focuses itself (OS raise) and broadcasts a matching claim. If a claim arrives the
// opener stands down; otherwise it opens the page locally.
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { isTauri } from "./client";

export const SINGLETON_QUERY_EVENT = "workbench:singleton_query";
export const SINGLETON_CLAIM_EVENT = "workbench:singleton_claim";

/** Minimal broadcast surface — real impl uses Tauri; tests inject a synchronous fake. */
export interface SingletonBus {
  emit: (event: string, payload: unknown) => Promise<void> | void;
  listen: <T>(event: string, cb: (payload: T) => void) => Promise<() => void>;
}

interface QueryPayload {
  page: string;
  nonce: string;
}

/** The real Tauri broadcast bus (no-op off-Tauri). */
export const tauriSingletonBus: SingletonBus = {
  emit: (event, payload) => {
    if (!isTauri()) return;
    void emit(event, payload);
  },
  listen: async (event, cb) => {
    if (!isTauri()) return () => {};
    return listen<unknown>(event, (e) => cb(e.payload as never));
  },
};

/** Raise this OS window (used by a window that holds a queried singleton). */
export async function focusCurrentWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await getCurrentWindow().setFocus();
  } catch {
    /* best-effort raise */
  }
}

/** Ask the other windows whether one already shows `page`. Resolves true if another window
 *  claims it (and focuses itself), false after `timeoutMs` with no claim. */
export function requestCrossWindowSingleton(
  page: string,
  bus: SingletonBus = tauriSingletonBus,
  timeoutMs = 150,
): Promise<boolean> {
  const nonce = `${Date.now()}_${Math.random().toString(36).slice(2)}`;
  return new Promise<boolean>((resolve) => {
    let settled = false;
    let unlisten: (() => void) | undefined;
    const finish = (claimed: boolean) => {
      if (settled) return;
      settled = true;
      unlisten?.();
      resolve(claimed);
    };
    void Promise.resolve(
      bus.listen<QueryPayload>(SINGLETON_CLAIM_EVENT, (p) => {
        if (p.page === page && p.nonce === nonce) finish(true);
      }),
    ).then((un) => {
      unlisten = un;
      if (settled) {
        un();
        return;
      }
      void bus.emit(SINGLETON_QUERY_EVENT, { page, nonce });
      setTimeout(() => finish(false), timeoutMs);
    });
  });
}

/** Wire this window to answer singleton queries: when asked about a page it currently shows,
 *  focus this window and claim it. Returns an unlisten disposer. */
export async function startSingletonResponder(
  hasPage: (page: string) => boolean,
  bus: SingletonBus = tauriSingletonBus,
  focusSelf: () => void | Promise<void> = focusCurrentWindow,
): Promise<() => void> {
  return bus.listen<QueryPayload>(SINGLETON_QUERY_EVENT, (p) => {
    if (!hasPage(p.page)) return;
    void focusSelf();
    void bus.emit(SINGLETON_CLAIM_EVENT, { page: p.page, nonce: p.nonce });
  });
}
