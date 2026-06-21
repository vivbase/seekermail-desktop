// Global external-link interceptor.
//
// Mail and other HTML bodies are injected with dangerouslySetInnerHTML
// (SanitizedMail, DraftPanel, Repository). A click on an <a href="https://…">
// inside them would otherwise navigate the app's own webview to that URL,
// replacing the entire SPA with the destination page. We intercept such clicks
// in the capture phase, cancel the default navigation, and hand the URL to the
// OS default browser / mail client via the open_external_url IPC command.
//
// Internal SPA links (same-origin http(s) routes and #fragment anchors) are left
// untouched so React Router keeps working — only cross-origin web links and
// mailto:/tel: links are externalised.
import { openExternalUrl } from "@/ipc/shell";

/** Non-http schemes we always hand to the OS default handler (lower-cased, with
 *  the trailing ':' as exposed by `HTMLAnchorElement.protocol`). */
const EXTERNAL_PROTOCOLS = new Set(["mailto:", "tel:"]);

/**
 * Decide whether a click on `anchor` should open externally. Returns the URL to
 * hand to the OS, or null when the click should fall through to default handling
 * (internal route, in-page anchor, or an unhandled scheme). Pure and exported so
 * it can be unit-tested without a DOM event.
 */
export function externalUrlForAnchor(
  anchor: HTMLAnchorElement,
  currentOrigin: string,
): string | null {
  // No destination → nothing to do (e.g. <a> used as a button).
  if (!anchor.getAttribute("href")) return null;

  // `protocol`/`origin`/`href` are the browser-resolved values, so relative and
  // fragment links resolve against the current document automatically.
  const protocol = anchor.protocol.toLowerCase();

  if (EXTERNAL_PROTOCOLS.has(protocol)) return anchor.href;

  if (protocol === "http:" || protocol === "https:") {
    // Same-origin http(s) is an internal route or asset — let the SPA handle it.
    return anchor.origin === currentOrigin ? null : anchor.href;
  }

  // javascript:, data:, blob:, file:, and pure #fragment links: leave default.
  return null;
}

function onAnchorClick(event: MouseEvent): void {
  // Respect handlers that already cancelled the event; ignore right-click (it
  // arrives as `auxclick` with button 2 and should keep the context menu).
  if (event.defaultPrevented) return;
  if (event.type === "auxclick" && event.button !== 1) return;

  const target = event.target as Element | null;
  const anchor = target?.closest?.("a[href]") as HTMLAnchorElement | null;
  if (!anchor) return;

  const url = externalUrlForAnchor(anchor, window.location.origin);
  if (!url) return;

  // Cancel the webview navigation, then open in the real browser instead.
  event.preventDefault();
  void openExternalUrl(url).catch(() => {
    // Opening failed (e.g. backend not ready): swallow. A no-op is strictly
    // better than letting the webview navigate away or throwing on a click.
  });
}

let installed = false;

/**
 * Install the singleton capture-phase link interceptor. Idempotent and safe to
 * call before React mounts. Capture phase ensures we run before React Router's
 * bubble-phase link handler and can cancel the navigation before the webview
 * acts on it; `auxclick` covers middle-click, which would request a new window.
 */
export function installExternalLinkHandler(): void {
  if (installed || typeof document === "undefined") return;
  installed = true;
  document.addEventListener("click", onAnchorClick, true);
  document.addEventListener("auxclick", onAnchorClick, true);
}
