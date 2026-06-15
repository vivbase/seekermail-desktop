// L1 filter drawer (T039, F_G2 §3, §4.1). Rendered as the left column inside the
// dashboard route — NOT mounted inside the shared Sidebar — to avoid modifying
// the shared layout component. It exposes system folders, unread/starred toggles,
// a tag placeholder, and the thread-folding toggle.
import { useTranslation } from "react-i18next";
import { useUi } from "@/stores/ui";
import { cn } from "@/lib/cn";

// ── System folder definitions ─────────────────────────────────────────────────

interface FolderNode {
  key: string;
  labelKey: string;
}

const SYSTEM_FOLDERS: FolderNode[] = [
  { key: "inbox", labelKey: "nav_inbox" },
  { key: "starred", labelKey: "nav_starred" },
  { key: "drafts", labelKey: "nav_drafts" },
  { key: "sent", labelKey: "nav_sent" },
  { key: "archive", labelKey: "nav_archive" },
  { key: "trash", labelKey: "nav_trash" },
  { key: "spam", labelKey: "nav_spam" },
];

// ── Folder node icons (inline SVG) ────────────────────────────────────────────

const FOLDER_ICONS: Record<string, React.ReactNode> = {
  inbox: (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="22 12 16 12 14 15 10 15 8 12 2 12" />
      <path d="M5.45 5.11L2 12v6a2 2 0 002 2h16a2 2 0 002-2v-6l-3.45-6.89A2 2 0 0016.76 4H7.24a2 2 0 00-1.79 1.11z" />
    </svg>
  ),
  starred: (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
    </svg>
  ),
  drafts: (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" />
      <path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" />
    </svg>
  ),
  sent: (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <line x1="22" y1="2" x2="11" y2="13" />
      <polygon points="22 2 15 22 11 13 2 9 22 2" />
    </svg>
  ),
  archive: (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="21 8 21 21 3 21 3 8" />
      <rect x="1" y="3" width="22" height="5" />
      <line x1="10" y1="12" x2="14" y2="12" />
    </svg>
  ),
  trash: (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6l-1 14H6L5 6" />
      <path d="M10 11v6M14 11v6" />
      <path d="M9 6V4h6v2" />
    </svg>
  ),
  spam: (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="10" />
      <line x1="12" y1="8" x2="12" y2="12" />
      <line x1="12" y1="16" x2="12.01" y2="16" />
    </svg>
  ),
};

// ── Component ─────────────────────────────────────────────────────────────────

export function FilterDrawer() {
  const { t } = useTranslation(["list", "nav"]);

  const mailFilter = useUi((s) => s.mailFilter);
  const setFolder = useUi((s) => s.setFolder);
  const setUnreadOnly = useUi((s) => s.setUnreadOnly);
  const setStarredOnly = useUi((s) => s.setStarredOnly);
  const threadFoldingEnabled = useUi((s) => s.threadFoldingEnabled);
  const setThreadFoldingEnabled = useUi((s) => s.setThreadFoldingEnabled);

  return (
    <nav
      aria-label={t("list:filter_title")}
      className="flex h-full w-52 shrink-0 flex-col gap-5 overflow-y-auto border-divider bg-parchment py-5 [border-inline-end-width:1px]"
    >
      {/* ── Quick filters ──────────────────────────────────────────────────── */}
      <div className="flex flex-col gap-1 px-3">
        <p className="section-label px-2 pb-1">{t("list:filter_title")}</p>

        <FilterToggle
          label={t("list:unread_only")}
          checked={mailFilter.unreadOnly}
          onChange={(v) => setUnreadOnly(v)}
        />
        <FilterToggle
          label={t("list:starred_only")}
          checked={mailFilter.starredOnly}
          onChange={(v) => setStarredOnly(v)}
        />
      </div>

      {/* ── System folders ─────────────────────────────────────────────────── */}
      <div className="flex flex-col gap-1 px-3">
        <p className="section-label px-2 pb-1">{t("list:folders_title")}</p>

        {/* All folders reset */}
        <FolderButton
          active={mailFilter.folder === null && !mailFilter.starredOnly}
          label={t("list:all_folders")}
          onClick={() => {
            setFolder(null);
            setStarredOnly(false);
          }}
        />

        {SYSTEM_FOLDERS.map((folder) => (
          <FolderButton
            key={folder.key}
            active={
              folder.key === "starred"
                ? mailFilter.starredOnly
                : mailFilter.folder === folder.key.toUpperCase()
            }
            label={t(`nav:${folder.labelKey}`, { defaultValue: folder.key })}
            icon={FOLDER_ICONS[folder.key]}
            onClick={() => {
              if (folder.key === "starred") {
                setFolder(null);
                setStarredOnly(true);
              } else {
                setFolder(folder.key.toUpperCase());
                setStarredOnly(false);
              }
            }}
          />
        ))}
      </div>

      {/* ── Tags (placeholder) ─────────────────────────────────────────────── */}
      <div className="flex flex-col gap-1 px-3">
        <p className="section-label px-2 pb-1">{t("nav:nav_tags", { defaultValue: "Tags" })}</p>
        <p className="px-2 font-body text-xs text-p7">
          {t("nav:nav_no_tags_yet", { defaultValue: "No tags yet" })}
        </p>
      </div>

      {/* ── AI Roles (placeholder) ─────────────────────────────────────────── */}
      <div className="flex flex-col gap-1 px-3">
        <p className="section-label px-2 pb-1">
          {t("nav:nav_ai_roles", { defaultValue: "AI Roles" })}
        </p>
        <button
          type="button"
          disabled
          aria-disabled="true"
          className="flex w-full cursor-not-allowed items-center gap-2 rounded-chip px-2 py-1.5 font-ui text-sm text-p7 opacity-50"
        >
          {t("nav:nav_coming_soon", { defaultValue: "Coming soon" })}
        </button>
      </div>

      {/* ── Thread folding toggle ──────────────────────────────────────────── */}
      <div className="mt-auto flex flex-col gap-2 border-t border-divider px-3 pt-4">
        <FilterToggle
          label={t("list:thread_folding")}
          checked={threadFoldingEnabled}
          onChange={(v) => setThreadFoldingEnabled(v)}
        />
      </div>
    </nav>
  );
}

// ── Sub-components ────────────────────────────────────────────────────────────

function FolderButton({
  active,
  label,
  icon,
  onClick,
}: {
  active: boolean;
  label: string;
  icon?: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-current={active ? "page" : undefined}
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-chip px-2 py-1.5 font-ui text-sm text-p9 transition-colors",
        "hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        active && "bg-p4 font-medium text-p10 [border-inline-start:2px_solid_var(--p9)]",
      )}
    >
      {icon && <span className="text-p7">{icon}</span>}
      {label}
    </button>
  );
}

function FilterToggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-center justify-between gap-2 rounded-chip px-2 py-1.5 hover:bg-p4">
      <span className="font-ui text-sm text-p9">{label}</span>
      <div
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        onKeyDown={(e) => {
          if (e.key === " " || e.key === "Enter") {
            e.preventDefault();
            onChange(!checked);
          }
        }}
        tabIndex={0}
        className={cn(
          "relative h-4 w-7 cursor-pointer rounded-avatar transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
          checked ? "bg-p9" : "bg-p5",
        )}
      >
        <span
          aria-hidden="true"
          className={cn(
            "absolute top-0.5 h-3 w-3 rounded-avatar bg-surface shadow transition-[inset-inline-start]",
            checked ? "start-3.5" : "start-0.5",
          )}
        />
      </div>
    </label>
  );
}
