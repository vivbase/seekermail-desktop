// Data Flow disclosure panel (T054, AI section upgraded by T069) — Settings →
// Data → Data Flow.
//
// The five non-AI rows stay FULLY STATIC: the panel documents where each data
// category goes under the local-first architecture. The AI section is dynamic
// (T069): `AiRoutingSection` reads the per-account effective AI routing over
// IPC and shows the real provider endpoints (dev/06 §8, ADR-0004) plus a 24h
// activity summary. T054's "No AI requests in v0.4" placeholder row is gone.
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

import { cn } from "@/lib/cn";

import AiRoutingSection from "./AiRoutingSection";

type Badge = "local" | "your_server" | "seekermail_cdn";

type DataFlowEntry = {
  /** i18n key for the data-category label. */
  key: string;
  /** i18n key for the destination description. */
  destination: string;
  badge: Badge;
  tls: boolean;
};

const DATA_FLOWS: DataFlowEntry[] = [
  {
    key: "data_flow_mail_content",
    destination: "data_flow_dest_local_device",
    badge: "local",
    tls: false,
  },
  {
    key: "data_flow_metadata",
    destination: "data_flow_dest_local_device",
    badge: "local",
    tls: false,
  },
  {
    key: "data_flow_vector_index",
    destination: "data_flow_dest_local_device",
    badge: "local",
    tls: false,
  },
  {
    key: "data_flow_imap_smtp",
    destination: "data_flow_dest_your_server",
    badge: "your_server",
    tls: true,
  },
  {
    key: "data_flow_update_check",
    destination: "data_flow_dest_seekermail",
    badge: "seekermail_cdn",
    tls: true,
  },
];

const BADGE_STYLES: Record<Badge, string> = {
  local: "bg-green text-p1",
  your_server: "bg-slate text-p1",
  seekermail_cdn: "bg-amber text-p10",
};

const BADGE_LABEL_KEYS: Record<Badge, string> = {
  local: "data_flow_badge_local",
  your_server: "data_flow_badge_your_server",
  seekermail_cdn: "data_flow_badge_seekermail_cdn",
};

/** Inline arrow icon — mirrored under RTL via CSS transform (T054 §6). */
function FlowArrow() {
  return (
    <svg
      aria-hidden
      viewBox="0 0 20 12"
      className="h-3 w-5 shrink-0 fill-none stroke-current text-p7 rtl:-scale-x-100"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M1 6h16M13 1l5 5-5 5" />
    </svg>
  );
}

export default function DataFlowPanel() {
  const { t } = useTranslation("settings");

  return (
    <div className="max-w-xl space-y-6">
      <div className="flex items-center justify-between">
        <p className="section-label">{t("data_flow_title")}</p>
        <Link to="/settings/data" className="font-ui text-xs text-p8 hover:text-p9">
          {t("export_back_to_data")}
        </Link>
      </div>

      <p className="font-body text-sm leading-relaxed text-p9">{t("data_flow_intro")}</p>

      <section className="flex flex-col gap-3">
        {DATA_FLOWS.map((entry) => (
          <div
            key={entry.key}
            className="flex items-center gap-3 rounded-card border border-divider bg-surface px-4 py-3"
          >
            <p className="w-32 shrink-0 font-ui text-xs font-medium uppercase tracking-wider text-p8">
              {t(entry.key)}
            </p>
            <FlowArrow />
            <p className="min-w-0 flex-1 font-body text-sm text-p9">
              {t(entry.destination)}
              {entry.tls && (
                <span className="ms-2 font-mono text-[10px] uppercase text-p7">
                  {t("data_flow_tls")}
                </span>
              )}
            </p>
            <span
              className={cn(
                "shrink-0 rounded-chip px-2 py-0.5 font-ui text-[10px] font-semibold uppercase tracking-wider",
                BADGE_STYLES[entry.badge],
              )}
            >
              {t(BADGE_LABEL_KEYS[entry.badge])}
            </span>
          </div>
        ))}
      </section>

      <AiRoutingSection />

      <p className="font-body text-xs text-p8">{t("data_flow_footer")}</p>
    </div>
  );
}
