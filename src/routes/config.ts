// The sidebar navigation inventory (07 §3) — mirrors the prototype's "Navigate"
// section (Dashboard · Inbox · Search · Team · Agents · Repository). Secondary
// surfaces (Pending, Unread, Processed, Report, GTE) are reached from the Dashboard
// cards, not the rail — exactly as the prototype does. "Search" is a first-class
// page (`/search`) so it follows the same route + active-dot logic as every other
// rail item.

export type SidebarItem = {
  kind: "route";
  path: string;
  navKey: string;
  // Only TEAM carries a red badge (unread + open decisions). AGENTS shows none:
  // a count of configured agents is not a notification.
  badge?: "team";
};

/** The left-rail items, in prototype order. */
export const SIDEBAR_ITEMS: SidebarItem[] = [
  { kind: "route", path: "/", navKey: "nav_dashboard" },
  { kind: "route", path: "/all-mail", navKey: "nav_inbox" },
  { kind: "route", path: "/search", navKey: "nav_search" },
  { kind: "route", path: "/team", navKey: "nav_team", badge: "team" },
  { kind: "route", path: "/agents", navKey: "nav_agents" },
  { kind: "route", path: "/repository", navKey: "nav_repository" },
];
