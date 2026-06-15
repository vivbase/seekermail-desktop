// OS push-notification helpers for Agent-IM events (T101). Lives inside `src/ipc/`
// so it may touch the Tauri API (the boundary rule). Notifications are best-effort:
// the body NEVER carries mail content (privacy, F_I2 §7), and a build without the
// notification plugin degrades to a silent no-op rather than throwing.
import { invoke } from "@tauri-apps/api/core";

import i18n from "@/i18n";

import { ipc, isTauri } from "./client";

export type NotificationLevel = "off" | "priority" | "all";

/** Read the global notification level (`app_settings`); defaults to "all". */
async function notificationLevel(): Promise<NotificationLevel> {
  try {
    const raw = await ipc("get_setting", { key: "notifications.global_level" });
    if (typeof raw !== "string") return "all";
    let value = raw;
    try {
      value = JSON.parse(raw) as string;
    } catch {
      // Stored as a bare string rather than JSON — use it as-is.
    }
    return value === "off" || value === "priority" ? value : "all";
  } catch {
    return "all";
  }
}

/** Pure gating rule for a new-query notification (unit-testable in isolation). */
export function shouldNotifyQuery(level: NotificationLevel, priority: string): boolean {
  if (level === "off") return false;
  if (level === "priority" && priority !== "high") return false;
  return true;
}

/** Best-effort OS notification; silent no-op off-Tauri or without the plugin. */
async function sendOsNotification(title: string, body: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("plugin:notification|notify", { options: { title, body } });
  } catch {
    // The notification plugin isn't registered in this build — degrade silently.
  }
}

/** High-priority `query:new` → OS notification (counts only, no mail content). */
export async function notifyQueryNew(payload: { priority: string }): Promise<void> {
  if (!shouldNotifyQuery(await notificationLevel(), payload.priority)) return;
  const t = i18n.getFixedT(null, "team");
  await sendOsNotification(t("notification_query_title"), t("notification_query_body"));
}

/** `risk:alert` → OS notification. The body is generic — the event payload
 *  carries no description and mail content is never surfaced. */
export async function notifyRiskAlert(): Promise<void> {
  if ((await notificationLevel()) === "off") return;
  const t = i18n.getFixedT(null, "team");
  await sendOsNotification(t("notification_risk_title"), t("notification_risk_body"));
}
