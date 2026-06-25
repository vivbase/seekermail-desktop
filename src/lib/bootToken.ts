// Boot-token codec for "open in new window" (WB-19/20 ↔ WB-12 boundary, Model S).
// A new OS window boots to one workspace tab via `?boot=<token>` (see commands/workbench.rs).
// The opener encodes a TabSpec into a URL-safe token; the new window decodes it on startup
// and calls openTab. base64url keeps the value free of characters that URLSearchParams would
// re-decode, so the round-trip is unambiguous.
import type { TabSpec } from "@/stores/workbench";

function b64urlEncode(s: string): string {
  const bin = Array.from(new TextEncoder().encode(s), (b) => String.fromCharCode(b)).join("");
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function b64urlDecode(token: string): string {
  const b64 = token.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(b64);
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

/** Encode a workspace tab spec into a URL-safe `?boot=` token. */
export function encodeBootToken(spec: TabSpec): string {
  return b64urlEncode(JSON.stringify(spec));
}

/** Decode a `?boot=` token back into a TabSpec; returns null for missing/garbled tokens. */
export function decodeBootToken(token: string): TabSpec | null {
  try {
    const parsed: unknown = JSON.parse(b64urlDecode(token));
    if (parsed && typeof parsed === "object" && "route" in parsed) {
      const route = (parsed as { route: unknown }).route;
      if (
        route &&
        typeof route === "object" &&
        typeof (route as { page?: unknown }).page === "string"
      ) {
        return parsed as TabSpec;
      }
    }
    return null;
  } catch {
    return null;
  }
}
