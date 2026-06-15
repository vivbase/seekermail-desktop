// AI routing disclosure (T069) — replaces T054's static "No AI requests in
// v0.4" placeholder. One row per account showing the configured provider and
// the *real* endpoint mail content goes to when AI runs (dev/06 §8, ADR-0004),
// plus a 24h activity summary built from `ai_decisions` — identifiers, counts,
// and token totals only, never content.
import { useTranslation } from "react-i18next";

import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import type { AiActivityRow, AiRouteEntry } from "@/ipc/dataFlow";
import { useDataFlowAiRouting } from "@/ipc/queries/dataFlow";

/** Display names for provider slugs (English UI copy, not translated keys). */
const PROVIDER_LABELS: Record<AiRouteEntry["aiProvider"], string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  ollama: "Ollama",
  local_onnx: "Local model (ONNX)",
  none: "",
};

function RouteRow({ route }: { route: AiRouteEntry }) {
  const { t } = useTranslation("settings");

  if (route.endpointKind === "none") {
    // AI off: nothing is sent anywhere — a muted row, no badge, no endpoint.
    return (
      <div className="flex items-center gap-3 rounded-card border border-divider bg-surface px-4 py-3 opacity-70">
        <AccountBadge route={route} />
        <div className="min-w-0 flex-1">
          <p className="truncate font-ui text-xs font-medium text-p9">{route.accountEmail}</p>
          <p className="mt-0.5 font-body text-sm text-p8">{t("data_flow_ai_disabled")}</p>
        </div>
      </div>
    );
  }

  const isCloud = route.endpointKind === "cloud";
  const destination =
    route.endpointKind === "in_process"
      ? t("data_flow_ai_in_process")
      : isCloud
        ? (route.endpointHost ?? route.endpointUrl ?? "")
        : t("data_flow_ai_on_device");

  return (
    <div className="flex items-start gap-3 rounded-card border border-divider bg-surface px-4 py-3">
      <AccountBadge route={route} />
      <div className="min-w-0 flex-1">
        <p className="truncate font-ui text-xs font-medium text-p9">{route.accountEmail}</p>
        <p className="mt-0.5 font-body text-sm text-p9">
          <span>{PROVIDER_LABELS[route.aiProvider]}</span>
          {route.aiModel && (
            <span className="ms-2 font-mono text-[10px] text-p7">{route.aiModel}</span>
          )}
        </p>
        <p className="mt-0.5 font-body text-sm text-p9">
          {isCloud && (
            <span className="me-2 font-ui text-[10px] font-semibold uppercase tracking-wider text-p7">
              {t("data_flow_ai_endpoint")}
            </span>
          )}
          <span className={isCloud ? "font-mono text-xs" : undefined}>{destination}</span>
        </p>
        {isCloud && (
          <p className="mt-1 font-body text-xs text-p8">{t("data_flow_ai_cloud_note")}</p>
        )}
      </div>
      <span
        className={cn(
          "shrink-0 rounded-chip px-2 py-0.5 font-ui text-[10px] font-semibold uppercase tracking-wider",
          isCloud ? "bg-slate text-p1" : "bg-green text-p1",
        )}
      >
        {isCloud ? t("data_flow_ai_cloud_badge") : t("data_flow_ai_local_badge")}
      </span>
    </div>
  );
}

function AccountBadge({ route }: { route: AiRouteEntry }) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex h-7 w-7 shrink-0 items-center justify-center rounded-avatar font-ui text-xs",
        accountColorClass(route.colorToken as AccountColorToken),
      )}
    >
      {route.accountEmail.charAt(0).toUpperCase()}
    </span>
  );
}

function ActivitySummary({
  activity,
  emailById,
}: {
  activity: AiActivityRow[];
  emailById: Map<string, string>;
}) {
  const { t } = useTranslation("settings");

  return (
    <div className="rounded-card border border-divider bg-surface px-4 py-4">
      <p className="font-ui text-xs font-medium uppercase tracking-wider text-p8">
        {t("data_flow_ai_activity_title")}
      </p>
      {activity.length === 0 ? (
        <p className="mt-2 font-body text-sm text-p8">{t("data_flow_ai_no_activity")}</p>
      ) : (
        <ul className="mt-2 flex flex-col gap-1.5">
          {activity.map((row) => (
            <li
              key={`${row.accountId}-${row.decisionType}-${row.aiModel ?? ""}`}
              className="flex flex-wrap items-baseline gap-x-2 font-body text-sm text-p9"
            >
              <span className="truncate font-ui text-xs text-p8">
                {emailById.get(row.accountId) ?? row.accountId}
              </span>
              <span className="font-mono text-xs text-p7">{row.decisionType}</span>
              {row.aiModel && <span className="font-mono text-xs text-p7">{row.aiModel}</span>}
              <span>{t("data_flow_ai_requests_count", { count: row.requestCount })}</span>
              <span className="text-p8">
                {t("data_flow_ai_tokens_count", {
                  count: row.inputTokens + row.outputTokens,
                })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * The dynamic AI section of the data-flow panel: per-account routing rows and
 * the 24h activity card. The fixed no-proxy statement (ADR-0004) always
 * renders, even while the routing query is in flight.
 */
export default function AiRoutingSection() {
  const { t } = useTranslation("settings");
  const { data } = useDataFlowAiRouting();

  const emailById = new Map((data?.routes ?? []).map((r) => [r.accountId, r.accountEmail]));

  return (
    <section className="flex flex-col gap-3">
      <p className="section-label">{t("data_flow_ai_routing_title")}</p>
      <p className="font-body text-sm leading-relaxed text-p9">
        {t("data_flow_no_seekermail_proxy")}
      </p>
      {data && (
        <>
          {data.routes.map((route) => (
            <RouteRow key={route.accountId} route={route} />
          ))}
          <ActivitySummary activity={data.activity} emailById={emailById} />
        </>
      )}
    </section>
  );
}
