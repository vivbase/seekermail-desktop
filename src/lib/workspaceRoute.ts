// Mapping between a workbench tab's WorkspaceRoute (Model S) and a URL path, so the active
// tab's route can mirror to the address bar for deep-linking / back-forward (18 §3, WB-09).
// Paths mirror the existing router (App.tsx): "/" dashboard, "/all-mail" inbox, "/team" agent_im, …
import type { WorkspaceRoute } from "@/stores/workbench";

/** The active tab's route → a URL path. */
export function routeToPath(route: WorkspaceRoute): string {
  switch (route.page) {
    case "dashboard":
      return "/";
    case "inbox":
      return route.params?.accountId ? `/accounts/${route.params.accountId}/mail` : "/all-mail";
    case "thread":
      return route.params?.mailId ? `/mail/${route.params.mailId}` : "/all-mail";
    case "compose":
      return "/compose";
    case "search":
      return "/search";
    case "pending":
      return "/pending";
    case "agent_im":
      return "/team";
    case "agents":
      return "/agents";
    case "repository":
      return "/repository";
    case "settings":
      return "/settings";
    default:
      return "/";
  }
}

/** A URL path → a WorkspaceRoute, or `null` if the path is not a workspace page (e.g. /onboarding). */
export function pathToRoute(pathname: string): WorkspaceRoute | null {
  const accountMail = /^\/accounts\/([^/]+)\/mail\/?$/.exec(pathname);
  if (accountMail?.[1]) return { page: "inbox", params: { accountId: accountMail[1] } };

  const mail = /^\/mail\/([^/]+)\/?$/.exec(pathname);
  if (mail?.[1]) return { page: "thread", params: { mailId: mail[1] } };

  if (pathname === "/") return { page: "dashboard" };
  if (pathname === "/all-mail") return { page: "inbox" };
  if (pathname === "/compose") return { page: "compose" };
  if (pathname === "/search") return { page: "search" };
  if (pathname === "/pending") return { page: "pending" };
  if (pathname === "/team") return { page: "agent_im" };
  if (pathname === "/agents") return { page: "agents" };
  if (pathname === "/repository") return { page: "repository" };
  if (pathname === "/settings" || pathname.startsWith("/settings/")) return { page: "settings" };

  return null;
}
