// SeekerMail ID card (A6, decoupled model) — the identity surface above the mailbox
// list. The SeekerMail ID is INDEPENDENT of mailboxes: created by signing in with
// Google, optional and local-first. Signing out clears only the identity; mailboxes
// and local mail are untouched. The marketing opt-in is shown only when signed in,
// defaults OFF, and is first-party only. Google sign-in is stubbed in the backend
// until the cloud-identity service ships (T121).
// Spec: knowledge base `docs/function list/F_A6_seekermail_id.md` §5 (rewritten).
import { useState } from "react";
import { useTranslation } from "react-i18next";

import ConfirmDialog from "@/components/ui/ConfirmDialog";
import {
  useGoogleSignIn,
  useSeekerMailId,
  useSetMarketingConsent,
  useSignOutSeekerMail,
} from "@/ipc/queries/identity";

export default function SeekerMailIdCard() {
  const { t } = useTranslation();
  const [confirm, setConfirm] = useState(false);
  const id = useSeekerMailId();
  const signOut = useSignOutSeekerMail();
  const signIn = useGoogleSignIn();
  const consent = useSetMarketingConsent();

  // While the identity loads, render nothing (avoids a flash of the wrong state).
  if (id.isLoading) return null;
  const account = id.data ?? null;

  return (
    <section className="mb-6 rounded-card border border-divider bg-surface p-5 shadow-card">
      <div className="flex flex-wrap items-center gap-4">
        <span
          aria-hidden
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-card bg-p9 font-ui text-sm font-bold tracking-wider text-white"
        >
          {t("acct_id_badge")}
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="font-display text-xl italic text-p10">{t("smid_title")}</h2>
          <p className="mt-0.5 truncate font-mono text-xs text-p8">
            {account ? (
              <>
                {t("smid_signed_in_as")}: <span className="text-p9">{account.email}</span>
              </>
            ) : (
              t("smid_signed_out")
            )}
          </p>
        </div>
        {account ? (
          <button
            type="button"
            onClick={() => setConfirm(true)}
            className="hover:bg-red/10 rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-red"
          >
            {t("acct_signout_id")}
          </button>
        ) : (
          <button
            type="button"
            onClick={() => signIn.mutate()}
            className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white"
          >
            {t("smid_signin_google")}
          </button>
        )}
      </div>

      {/* Marketing opt-in — only when signed in. Default OFF; first-party only. */}
      {account && (
        <label className="mt-4 flex items-center gap-2 border-t border-divider pt-4 font-body text-xs text-p8">
          <input
            type="checkbox"
            checked={account.marketingConsent}
            onChange={(e) => consent.mutate({ consent: e.target.checked, source: "settings" })}
          />
          {t("smid_marketing_optin")}
        </label>
      )}

      <p className="mt-4 font-body text-xs leading-relaxed text-p8">{t("smid_caption")}</p>

      {signIn.isError && (
        <p role="status" className="mt-2 font-ui text-xs text-amber">
          {t("smid_signin_pending")}
        </p>
      )}
      {signOut.isError && (
        <p role="alert" className="mt-2 font-ui text-xs text-red">
          {t("acct_signout_id_error")}
        </p>
      )}

      <ConfirmDialog
        open={confirm}
        title={t("acct_signout_id_title")}
        body={t("acct_signout_id_body")}
        destructive
        confirmLabel={t("acct_signout_id")}
        onConfirm={() => signOut.mutate(undefined, { onSettled: () => setConfirm(false) })}
        onCancel={() => setConfirm(false)}
      />
    </section>
  );
}
