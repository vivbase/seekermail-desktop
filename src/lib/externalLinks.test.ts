// External-link interceptor: cross-origin web links and mailto:/tel: open in the
// OS browser; internal SPA routes and in-page anchors fall through to the router.
import { describe, it, expect } from "vitest";

import { externalUrlForAnchor, installExternalLinkHandler } from "./externalLinks";

function anchor(href: string | null): HTMLAnchorElement {
  const a = document.createElement("a");
  if (href !== null) a.setAttribute("href", href);
  return a;
}

const ORIGIN = window.location.origin;

describe("externalUrlForAnchor", () => {
  it("externalises a cross-origin web link", () => {
    const a = anchor("https://accounts.google.com/signin?next=/console");
    expect(externalUrlForAnchor(a, ORIGIN)).toBe(
      "https://accounts.google.com/signin?next=/console",
    );
  });

  it("externalises mailto: and tel: links regardless of origin", () => {
    expect(externalUrlForAnchor(anchor("mailto:alice@example.com"), ORIGIN)).toBe(
      "mailto:alice@example.com",
    );
    expect(externalUrlForAnchor(anchor("tel:+15550100"), ORIGIN)).toBe("tel:+15550100");
  });

  it("leaves same-origin routes and in-page anchors to the SPA", () => {
    expect(externalUrlForAnchor(anchor("/inbox"), ORIGIN)).toBeNull();
    expect(externalUrlForAnchor(anchor(`${ORIGIN}/agents`), ORIGIN)).toBeNull();
    expect(externalUrlForAnchor(anchor("#section"), ORIGIN)).toBeNull();
  });

  it("ignores javascript: and other non-navigational schemes", () => {
    expect(externalUrlForAnchor(anchor("javascript:void(0)"), ORIGIN)).toBeNull();
    expect(externalUrlForAnchor(anchor("data:text/html,<b>x</b>"), ORIGIN)).toBeNull();
  });

  it("ignores an anchor with no href", () => {
    expect(externalUrlForAnchor(anchor(null), ORIGIN)).toBeNull();
  });
});

describe("installExternalLinkHandler", () => {
  it("cancels the default navigation for an external link click", () => {
    installExternalLinkHandler();
    const a = anchor("https://accounts.google.com/signin");
    document.body.appendChild(a);
    const event = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 });
    a.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    a.remove();
  });

  it("does not touch a same-origin route click", () => {
    installExternalLinkHandler();
    const a = anchor("/inbox");
    document.body.appendChild(a);
    const event = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 });
    a.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    a.remove();
  });
});
