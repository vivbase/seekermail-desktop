// Accounts settings list (T017). Lists accounts with four-state badges and hosts
// the add-account wizard. The SeekerMail ID card sits on top and is INDEPENDENT of
// the mailbox list (A6, decoupled model) — it renders whether or not any mailbox
// exists. Server state via TanStack Query hooks.
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useAccounts } from "@/ipc/queries/accounts";
import SeekerMailIdCard from "@/components/account/SeekerMailIdCard";
import AccountRow from "./AccountRow";
import AddAccountWizard from "./AddAccountWizard";

export default function AccountList() {
  const { t } = useTranslation();
  const accounts = useAccounts();
  const [wizardOpen, setWizardOpen] = useState(false);

  const list = accounts.data ?? [];

  return (
    <section className="mx-auto max-w-2xl p-6">
      <header className="mb-4 flex items-center justify-between">
        <h1 className="font-display text-2xl italic text-p10">{t("acct_settings_title")}</h1>
        <button
          type="button"
          onClick={() => setWizardOpen(true)}
          className="rounded-chip bg-p9 px-4 py-2 font-ui text-xs uppercase tracking-wider text-white"
        >
          {t("acct_add")}
        </button>
      </header>

      {/* Identity is independent of mailboxes — show the ID card regardless. */}
      <SeekerMailIdCard />

      {accounts.isLoading ? (
        <p className="font-body text-sm text-p8">{t("state_loading")}</p>
      ) : list.length === 0 ? (
        <p className="font-body text-sm text-p8">{t("acct_empty")}</p>
      ) : (
        <ul className="space-y-3">
          {list.map((account) => (
            <AccountRow key={account.id} account={account} />
          ))}
        </ul>
      )}

      {wizardOpen && <AddAccountWizard onClose={() => setWizardOpen(false)} />}
    </section>
  );
}
