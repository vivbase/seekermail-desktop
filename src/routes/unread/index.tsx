// Unread mail route (T047). Fills the /unread stub with a real filtered list.
// Reuses <ThreadList /> — the component reads mailFilter.unreadOnly from the
// shared store and passes isUnread=true to useMailsInfinite when set.
// Focus lands on the <h1> on mount (dev/11 §3).
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import { ThreadList } from "@/components/mail/ThreadList";
import { useUi } from "@/stores/ui";

export default function Unread() {
  const { t } = useTranslation("settings");
  const headingRef = useRef<HTMLHeadingElement>(null);

  const setUnreadOnly = useUi((s) => s.setUnreadOnly);

  // Force the shared unread filter while this route is mounted.
  // Restore to false on unmount so other routes are unaffected.
  useEffect(() => {
    setUnreadOnly(true);
    return () => {
      setUnreadOnly(false);
    };
  }, [setUnreadOnly]);

  // Programmatic focus for accessibility (dev/11 §3).
  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  return (
    <div className="flex h-full flex-col">
      <header className="shrink-0 border-b border-divider px-6 py-5">
        <h1
          ref={headingRef}
          tabIndex={-1}
          className="font-display text-3xl italic text-p10 outline-none"
        >
          {t("list_page_unread")}
        </h1>
      </header>

      {/* ThreadList picks up mailFilter.unreadOnly=true from the store and
          renders unread mail across all accounts (accountId=null). */}
      <div className="min-h-0 flex-1 overflow-hidden">
        <ThreadList accountId={null} />
      </div>
    </div>
  );
}
