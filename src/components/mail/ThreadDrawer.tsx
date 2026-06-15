// Collapsible right-side thread drawer for the L2 reading view (T041).
// Lists all messages in the current thread; the active message is highlighted.
// When the thread has only a single message (the common case for the mock data)
// it renders gracefully with a header showing "1 message".
// Clicking another message navigates to /mail/:id.
// The Legal tab hosts the D1 analysis sidebar (T071) — it replaced the T041
// "AI Assistant" placeholder; D2 (business) joins as a sibling tab in v0.6.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import type { MailDetail } from "@shared/bindings";

import { formatMailDate } from "@/lib/formatDate";
import { cn } from "@/lib/cn";
import { LegalSidebar } from "./LegalSidebar";

interface ThreadDrawerProps {
  /** The currently open mail — used as the single source of truth for thread data.
   *  Since the mock layer exposes only one MailDetail, this component builds a
   *  synthetic single-item thread list from the current mail and renders robustly.
   *  When the backend thread-list query lands, replace with real data. */
  currentMail: MailDetail;
  isOpen: boolean;
  onClose: () => void;
}

// ── Tab ids ────────────────────────────────────────────────────────────────────

type DrawerTab = "thread" | "legal";

export function ThreadDrawer({ currentMail, isOpen, onClose }: ThreadDrawerProps) {
  const { t } = useTranslation(["reading", "legal"]);
  const navigate = useNavigate();
  const [activeTab, setActiveTab] = useState<DrawerTab>("thread");

  if (!isOpen) return null;

  // Synthetic single-item thread from the current mail (real query lands v0.5+).
  const threadItems = [
    {
      id: currentMail.id,
      fromName: currentMail.fromName ?? currentMail.fromEmail,
      dateSent: currentMail.dateSent,
      isCurrent: true,
      isRead: currentMail.isRead,
      snippet: null as string | null,
    },
  ];

  const threadCount = threadItems.length;

  return (
    <aside
      aria-label={t("show_thread")}
      className="flex h-full w-64 shrink-0 flex-col border-s border-divider bg-surface"
    >
      {/* Drawer header */}
      <div className="flex items-center justify-between border-b border-divider px-4 py-3">
        <p className="section-label">{t("thread_count", { count: threadCount })}</p>
        <button
          type="button"
          onClick={onClose}
          aria-label={t("hide_thread")}
          className="rounded-chip p-1 text-p7 transition-colors hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
            aria-hidden="true"
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4 4 12M4 4l8 8" />
          </svg>
        </button>
      </div>

      {/* Tab bar */}
      <div className="flex border-b border-divider" role="tablist">
        <button
          role="tab"
          type="button"
          aria-selected={activeTab === "thread"}
          onClick={() => setActiveTab("thread")}
          className={cn(
            "flex-1 py-2 font-ui text-[10px] uppercase tracking-wider transition-colors",
            activeTab === "thread" ? "border-b-2 border-p9 text-p9" : "text-p7 hover:text-p9",
          )}
        >
          Thread
        </button>
        <button
          role="tab"
          type="button"
          aria-selected={activeTab === "legal"}
          onClick={() => setActiveTab("legal")}
          className={cn(
            "flex-1 py-2 font-ui text-[10px] uppercase tracking-wider transition-colors",
            activeTab === "legal" ? "border-b-2 border-p9 text-p9" : "text-p7 hover:text-p9",
          )}
        >
          {t("legal:legal_tab_label")}
        </button>
      </div>

      {/* Tab panels */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {activeTab === "thread" && (
          <ul role="list" className="divide-y divide-divider">
            {threadItems.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  aria-current={item.isCurrent ? "true" : undefined}
                  onClick={() => {
                    if (!item.isCurrent) navigate(`/mail/${item.id}`);
                  }}
                  className={cn(
                    "w-full px-4 py-3 text-start transition-colors",
                    item.isCurrent
                      ? "cursor-default bg-p4"
                      : "hover:bg-p4 focus:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-p9",
                  )}
                >
                  <div className="flex items-baseline justify-between gap-2">
                    <span
                      className={cn(
                        "truncate font-ui text-xs",
                        item.isRead ? "text-p8" : "font-semibold text-p10",
                      )}
                    >
                      {item.fromName}
                    </span>
                    <span className="shrink-0 font-mono text-[10px] text-p7">
                      {formatMailDate(item.dateSent)}
                    </span>
                  </div>
                  {item.snippet && (
                    <p className="mt-0.5 line-clamp-1 font-body text-[11px] text-p7">
                      {item.snippet}
                    </p>
                  )}
                  {item.isCurrent && (
                    <span className="mt-1 inline-flex items-center rounded-chip bg-p9 px-1.5 py-0.5 font-ui text-[9px] uppercase tracking-wider text-white">
                      Current
                    </span>
                  )}
                </button>
              </li>
            ))}
          </ul>
        )}

        {activeTab === "legal" && <LegalSidebar mailId={currentMail.id} />}
      </div>
    </aside>
  );
}
