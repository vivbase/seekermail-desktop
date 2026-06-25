// Left navigation rail (07 §3) — rebuilt to match the prototype shell exactly: a
// two-line "Post / SeekerMail" brand, a single "Navigate" section, a full-width
// Compose button, and an account footer that opens Profile. Active state follows
// the router (NavLink); every rail item — including Search (/search) — is a route.
// Uses the `.sidebar` / `.nav-item` class hooks so RTL locales mirror (scripts.css).
import { NavLink, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import { useTeamUnreadCount } from "@/ipc/queries/im";
import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import { SIDEBAR_ITEMS } from "@/routes/config";
import { openSpecAttr } from "@/lib/openSpec";
import { pathToRoute } from "@/lib/workspaceRoute";

import GetMailButton from "./GetMailButton";

/** Right-click "open in new tab" spec for a sidebar route (WB-19). */
function navOpenSpec(path: string): Record<string, string> {
  const route = pathToRoute(path);
  return route ? openSpecAttr({ route }) : {};
}

/** Active indicator: a red dot when active, an empty 7px spacer otherwise. */
function NavDot({ active }: { active: boolean }) {
  return active ? (
    <span aria-hidden className="h-[7px] w-[7px] shrink-0 rounded-avatar bg-red" />
  ) : (
    <span aria-hidden className="w-[7px] shrink-0" />
  );
}

const LABEL = "flex-1 font-ui text-[11px] uppercase tracking-[0.07em]";
const BADGE = "rounded-[10px] bg-red px-1.5 py-px font-mono text-[9px] leading-none text-white";

export default function Sidebar() {
  const { t } = useTranslation(["nav", "common", "team"]);
  const navigate = useNavigate();

  // T101: red badge on the TEAM item — unread agent messages plus unresolved
  // decision cards. Opening the channel clears the unread half; open decisions
  // persist until answered/skipped. AGENTS carries no badge: an agent count is
  // not a notification, so a red dot there only ever read as noise.
  const { data: teamUnread } = useTeamUnreadCount();
  const teamCount = teamUnread ?? 0;

  // Account footer reads the primary account (falls back to the first one).
  const { data: accounts } = useAccounts();
  const primary = (accounts ?? []).find((a) => a.isPrimary) ?? (accounts ?? [])[0] ?? null;

  return (
    <aside className="sidebar flex h-full w-[200px] shrink-0 flex-col border-divider bg-parchment px-5 pb-5 pt-7 [border-inline-end-width:1px]">
      <nav className="flex flex-1 flex-col overflow-y-auto">
        <div className="mb-2 flex items-center justify-between">
          <p className="font-ui text-[9px] font-semibold uppercase tracking-[0.1em] text-p7">
            {t("nav:nav_navigate")}
          </p>
          <GetMailButton />
        </div>

        {SIDEBAR_ITEMS.map((item) => (
          <NavLink
            key={item.path}
            to={item.path}
            end={item.path === "/"}
            {...navOpenSpec(item.path)}
            className="nav-item flex items-center gap-[7px] py-[5px] transition-opacity hover:opacity-70"
          >
            {({ isActive }) => (
              <>
                <NavDot active={isActive} />
                <span className={cn(LABEL, isActive ? "font-semibold text-p10" : "text-p8")}>
                  {t(`nav:${item.navKey}`)}
                </span>
                {item.badge === "team" && teamCount > 0 && (
                  <span
                    aria-label={t("team:team_badge_aria", { count: teamCount })}
                    className={BADGE}
                  >
                    {teamCount > 9 ? "9+" : teamCount}
                  </span>
                )}
              </>
            )}
          </NavLink>
        ))}

        {/* Compose — full-width dark button (prototype) */}
        <button
          type="button"
          onClick={() => navigate("/compose")}
          className="mt-2.5 flex w-full items-center justify-center gap-1.5 rounded-chip bg-p10 px-3 py-2 font-ui text-[9px] font-bold uppercase tracking-[0.09em] text-white transition-colors hover:bg-p9"
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
            <path d="M5 1v8M1 5h8" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
          </svg>
          {t("nav:nav_compose")}
        </button>
      </nav>

      {/* Account footer → Profile (prototype sb-foot) */}
      {primary && (
        <button
          type="button"
          onClick={() => navigate("/profile")}
          title={t("common:profile_desc")}
          className="sb-foot mt-auto flex items-center gap-2.5 border-divider pt-4 text-start transition-opacity [border-block-start-width:1px] hover:opacity-70"
        >
          <span
            className={cn(
              "flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-chip font-ui text-[10px] font-bold",
              accountColorClass(primary.colorToken as AccountColorToken),
            )}
          >
            {primary.badgeLabel}
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate font-body text-[11px] leading-snug text-p10">
              {primary.displayName}
            </span>
            <span className="block truncate font-ui text-[9px] uppercase tracking-[0.06em] text-p8">
              {primary.email}
              {primary.isPrimary ? ` · ${t("common:sidebar_primary")}` : ""}
            </span>
          </span>
          <svg
            width="12"
            height="12"
            viewBox="0 0 12 12"
            fill="none"
            aria-hidden
            className="shrink-0 opacity-40"
          >
            <circle cx="6" cy="6" r="5" stroke="currentColor" strokeWidth="1.2" />
            <path
              d="M6 4v2.5l1.5 1.5"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
            />
          </svg>
        </button>
      )}
    </aside>
  );
}
