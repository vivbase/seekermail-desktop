// Tracker status badge (T029). Green when clean, amber with a count + expandable
// panel when trackers were blocked. Token colors only; copy via i18n.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TrackerInfo } from "@shared/bindings";

import { useAllowRemoteImages } from "@/ipc/queries/mail";

interface TrackerBadgeProps {
  info: TrackerInfo;
}

export default function TrackerBadge({ info }: TrackerBadgeProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const allow = useAllowRemoteImages();

  if (info.trackerCount === 0 && !info.blocked) {
    return (
      <span className="bg-green/15 inline-flex items-center gap-1 rounded-chip px-2 py-1 font-ui text-[10px] uppercase tracking-wider text-green">
        {t("tracker_none")}
      </span>
    );
  }

  return (
    <div className="mb-3">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="bg-amber/15 inline-flex items-center gap-1 rounded-chip px-2 py-1 font-ui text-[10px] uppercase tracking-wider text-amber"
        aria-expanded={open}
      >
        {t("tracker_blocked_n", { count: info.trackerCount })}
      </button>

      {open && (
        <div className="mt-2 rounded-card border border-divider bg-surface p-3 shadow-card">
          <p className="font-ui text-[10px] uppercase tracking-wider text-p8">
            {t("tracker_panel_title")}
          </p>
          <button
            type="button"
            disabled={info.imagesAllowed || allow.isPending}
            onClick={() =>
              allow.mutate({
                mailId: "",
                scope: { type: "alwaysSender", senderEmail: info.senderEmail },
              })
            }
            className="mt-2 rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white disabled:opacity-50"
          >
            {t("tracker_panel_allow_sender")}
          </button>
        </div>
      )}
    </div>
  );
}
