// One account row + expandable panel (T017). Color badge, four-state indicator,
// sync range / history progress / disk usage, and row actions. IPC via hooks.
//
// A6 decoupled model: a mailbox is just a data source. "Remove" deletes only this
// mailbox and its local copy (the last one included — no more "can't remove your
// only account" dead-end). Signing out of the SeekerMail ID is a separate action on
// the ID card, independent of mailboxes.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Account } from "@shared/bindings";

import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import {
  useBackfillStatus,
  useDeleteAccount,
  useDisableAccount,
  useDiskUsage,
  useEnableAccount,
  useSyncState,
} from "@/ipc/queries/accounts";
import ConfirmDialog from "@/components/ui/ConfirmDialog";
import ProgressBar from "@/components/ui/ProgressBar";
import { showToast } from "@/components/ui/Toast";
import EditAccountSheet from "./EditAccountSheet";

type AccountStatus = "active" | "disabled" | "auth_failed" | "sync_error";

interface AccountRowProps {
  account: Account;
}

export default function AccountRow({ account }: AccountRowProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);

  const sync = useSyncState(account.id);
  const disk = useDiskUsage(account.id);
  const backfill = useBackfillStatus(account.id);
  const enable = useEnableAccount();
  const disable = useDisableAccount();
  const del = useDeleteAccount();

  const status = deriveStatus(account, sync.data?.lastSyncResult ?? null);
  const dotClass = STATUS_DOT[status];

  const rangeText =
    account.knowledgeDepthMonths == null
      ? t("depth_all")
      : t("depth_months", { months: account.knowledgeDepthMonths });

  const bf = backfill.data;
  const progressPct =
    bf && bf.totalUidCount && bf.totalUidCount > 0 ? (bf.fetchedCount / bf.totalUidCount) * 100 : 0;

  return (
    <li
      className={`rounded-card border border-divider ${status === "disabled" ? "opacity-60" : ""}`}
    >
      <div className="flex items-center gap-3 p-4">
        <span
          className={`flex h-9 w-9 items-center justify-center rounded-avatar font-ui text-sm ${accountColorClass(
            account.colorToken as AccountColorToken,
          )}`}
        >
          {account.badgeLabel}
        </span>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="min-w-0 flex-1 text-start"
        >
          <span className="block truncate font-body text-sm text-p10">{account.displayName}</span>
          <span className="block truncate font-mono text-xs text-p8">{account.email}</span>
        </button>
        <span className="flex items-center gap-1.5">
          <span className={`h-2 w-2 rounded-avatar ${dotClass}`} aria-hidden />
          <span className="font-ui text-[10px] uppercase tracking-wider text-p8">
            {t(STATUS_LABEL[status])}
          </span>
        </span>
      </div>

      {open && (
        <div className="space-y-3 border-t border-divider px-4 py-3">
          <Row label={t("acct_sync_range")} value={rangeText} />
          <div>
            <p className="font-ui text-[10px] uppercase tracking-wider text-p8">
              {t("acct_history_progress")}
            </p>
            <div className="mt-1">
              <ProgressBar percent={progressPct} accentClass={`bg-${account.colorToken}`} />
            </div>
            <p className="mt-1 font-mono text-xs text-p8">
              {bf ? t("depth_count", { count: bf.fetchedCount }) : t("acct_disk_calculating")}
            </p>
          </div>
          <Row
            label={t("acct_disk_usage")}
            value={
              disk.data
                ? t("depth_size", { mb: Math.round(disk.data.totalBytes / (1024 * 1024)) })
                : t("acct_disk_calculating")
            }
          />
          <div className="flex flex-wrap gap-2 pt-1">
            {account.isActive ? (
              <Action label={t("acct_disable")} onClick={() => disable.mutate(account.id)} />
            ) : (
              <Action label={t("acct_enable")} onClick={() => enable.mutate(account.id)} />
            )}
            <Action label={t("acct_edit")} onClick={() => setEditing(true)} />
            <Action
              label={t("acct_signout")}
              destructive
              onClick={() => setConfirmDisconnect(true)}
            />
          </div>
        </div>
      )}

      {editing && (
        <EditAccountSheet
          account={account}
          needsReauth={status === "auth_failed"}
          onClose={() => setEditing(false)}
        />
      )}

      {/* A6 decoupled: removing a mailbox deletes only that data source — the
          last one included. The SeekerMail ID (if any) is unaffected. */}
      <ConfirmDialog
        open={confirmDisconnect}
        title={t("acct_signout_mbx_title", { name: account.displayName })}
        body={t("acct_signout_mbx_body", { name: account.displayName, email: account.email })}
        destructive
        confirmLabel={t("acct_signout")}
        pending={del.isPending}
        pendingLabel={t("acct_removing")}
        onConfirm={() => {
          // Drive the dialog off the mutation result instead of closing optimistically:
          // close + confirm on success, surface a toast on failure (the dialog stays
          // open so the user can retry — the backend self-heals a corrupt FTS index).
          del.mutate(account.id, {
            onSuccess: () => {
              setConfirmDisconnect(false);
              showToast(t("acct_remove_done", { name: account.displayName }));
            },
            onError: () => showToast(t("acct_remove_failed")),
          });
        }}
        onCancel={() => setConfirmDisconnect(false)}
      />
    </li>
  );
}

function deriveStatus(account: Account, lastSyncResult: string | null): AccountStatus {
  if (!account.isActive) return "disabled";
  if (lastSyncResult === "auth_error") return "auth_failed";
  if (lastSyncResult === "network_error") return "sync_error";
  return "active";
}

const STATUS_DOT: Record<AccountStatus, string> = {
  active: "bg-green",
  disabled: "bg-p6",
  auth_failed: "bg-red",
  sync_error: "bg-amber",
};

const STATUS_LABEL: Record<AccountStatus, string> = {
  active: "acct_state_active",
  disabled: "acct_state_disabled",
  auth_failed: "acct_state_auth_failed",
  sync_error: "acct_state_sync_error",
};

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="font-ui text-[10px] uppercase tracking-wider text-p8">{label}</span>
      <span className="font-body text-sm text-p9">{value}</span>
    </div>
  );
}

function Action({
  label,
  onClick,
  destructive,
}: {
  label: string;
  onClick: () => void;
  destructive?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider ${
        destructive ? "hover:bg-red/10 text-red" : "text-p8 hover:bg-p4"
      }`}
    >
      {label}
    </button>
  );
}
