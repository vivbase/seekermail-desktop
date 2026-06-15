// Settings left-side category navigation (T049). Uses NavLink so the active
// route drives the highlight without extra store state. Uses logical CSS
// properties for RTL compatibility (dev/07 §7, CLAUDE.md RTL section).
import { NavLink } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/cn";

interface SettingsNavItem {
  path: string;
  labelKey: string;
}

const NAV_ITEMS: SettingsNavItem[] = [
  { path: "/settings/accounts", labelKey: "nav_accounts" },
  { path: "/settings/appearance", labelKey: "nav_appearance" },
  { path: "/settings/privacy", labelKey: "nav_privacy" },
  { path: "/settings/ai", labelKey: "nav_ai" },
  { path: "/settings/data", labelKey: "nav_data" },
  { path: "/settings/about", labelKey: "nav_about" },
];

export function SettingsNav() {
  const { t } = useTranslation("settings");

  return (
    <nav
      aria-label={t("title")}
      className="flex h-full w-44 shrink-0 flex-col gap-1 border-divider py-5 [border-inline-end-width:1px]"
    >
      <p className="section-label mb-2 px-4">{t("title")}</p>
      {NAV_ITEMS.map((item) => (
        <NavLink
          key={item.path}
          to={item.path}
          className={({ isActive }) =>
            cn(
              "nav-item mx-2 flex items-center rounded-chip px-3 py-2 font-ui text-sm transition-colors",
              isActive ? "bg-p4 font-medium text-p10" : "text-p9 hover:bg-p4",
            )
          }
          aria-current={undefined /* NavLink sets this automatically */}
        >
          {t(item.labelKey)}
        </NavLink>
      ))}
    </nav>
  );
}
