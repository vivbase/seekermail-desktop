// Reading-view image helpers (B1/B2/F_G3 §4.1). Pure DOM utilities that run
// AFTER DOMPurify has injected the sanitised body, so they never widen the
// sanitiser's surface: they only rewrite the `src` of <img> nodes already in the
// DOM to `data:` URIs (allowed by CSP `img-src data:`). Neither helper imports
// `@tauri-apps/api` — the network call is passed in from a queries/ hook, keeping
// the single-data-access-layer rule intact (07 §6).
import type { InlineImage, RemoteImage } from "@shared/bindings";

/** Strip a `cid:` reference down to its bare Content-ID for matching: drop the
 *  scheme, URL-decode, remove angle brackets, lower-case. Mirrors the backend's
 *  `normalize_cid` so both ends agree on the key. */
function cidKey(src: string): string {
  const raw = src.replace(/^cid:/i, "");
  let decoded = raw;
  try {
    decoded = decodeURIComponent(raw);
  } catch {
    // Malformed escapes: fall back to the raw value.
  }
  return decoded.replace(/^<|>$/g, "").trim().toLowerCase();
}

/**
 * Swap every `<img src="cid:…">` in `root` to a `data:` URI built from the
 * resolved inline images. Inline parts ship inside the message, so this is
 * always safe to do automatically — there is no privacy cost (F_G3 §4.1).
 * Unresolved `cid:` images are left as-is (CSS keeps them hidden so no broken
 * frame shows).
 */
export function applyInlineImages(
  root: HTMLElement | null,
  images: InlineImage[] | undefined,
): void {
  if (!root || !images || images.length === 0) return;
  const byCid = new Map(images.map((img) => [img.contentId.trim().toLowerCase(), img]));
  root.querySelectorAll<HTMLImageElement>("img").forEach((img) => {
    const src = img.getAttribute("src") ?? "";
    if (!/^cid:/i.test(src)) return;
    const hit = byCid.get(cidKey(src));
    if (hit) img.setAttribute("src", `data:${hit.mime};base64,${hit.dataBase64}`);
  });
}

/**
 * Reveal blocked remote images by fetching each through the backend (no cookies
 * / Referer / User-Agent) and swapping the stashed `data-remote-src` to a
 * `data:` URI. The webview itself never connects to the origin. Per-image
 * failures are swallowed so the rest still load. Returns the number revealed.
 */
export async function revealRemoteImages(
  root: HTMLElement | null,
  fetchImage: (url: string) => Promise<RemoteImage>,
): Promise<number> {
  if (!root) return 0;
  const nodes = Array.from(root.querySelectorAll<HTMLElement>("[data-remote-src]"));
  let revealed = 0;
  await Promise.all(
    nodes.map(async (el) => {
      const url = el.getAttribute("data-remote-src");
      if (!url) return;
      try {
        const { mime, dataBase64 } = await fetchImage(url);
        el.setAttribute("src", `data:${mime};base64,${dataBase64}`);
        el.removeAttribute("data-remote-src");
        revealed += 1;
      } catch {
        // Leave this image blocked; the bar/notice stays accurate.
      }
    }),
  );
  return revealed;
}
