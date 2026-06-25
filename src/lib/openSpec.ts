// "Open in new tab/window" affordance plumbing (WB-19). A surface marks an element with a
// `data-open-spec` attribute describing the workspace it represents; the global
// WorkbenchContextMenu reads the nearest one on right-click. Keeping it as a data attribute
// means surfaces add ONE line (the spread) instead of wiring their own menu state.
import type { TabSpec } from "@/stores/workbench";

export const OPEN_SPEC_ATTR = "data-open-spec";

/** Spread onto an element so right-click offers "open in new tab" for this workspace. */
export function openSpecAttr(spec: TabSpec): Record<string, string> {
  return { [OPEN_SPEC_ATTR]: JSON.stringify(spec) };
}

/** Parse a `data-open-spec` value into a TabSpec; null if missing/garbled. */
export function parseOpenSpec(value: string | null | undefined): TabSpec | null {
  if (!value) return null;
  try {
    const parsed: unknown = JSON.parse(value);
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
