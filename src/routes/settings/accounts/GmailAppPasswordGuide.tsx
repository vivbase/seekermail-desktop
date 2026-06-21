// Gmail App Password guide. Gmail connects through IMAP + an App Password rather
// than OAuth (the Google mail scope is restricted/paid — knowledge base
// analysis/29), and the most common reason Gmail users abandon setup is not
// knowing how to obtain that password. This inline guide is rendered in the
// add-account wizard's credentials step the moment a Gmail address is entered.
//
// The "Open Google App Passwords" action is handed to the OS browser via the
// open_external_url IPC command (never navigates the app webview).
import { useTranslation } from "react-i18next";

import { openExternalUrl } from "@/ipc/shell";

const APP_PASSWORDS_URL = "https://myaccount.google.com/apppasswords";

export default function GmailAppPasswordGuide() {
  const { t } = useTranslation();
  const steps = [t("gmailGuide_step1"), t("gmailGuide_step2"), t("gmailGuide_step3")];

  return (
    <section
      aria-label={t("gmailGuide_title")}
      className="rounded-card border border-divider bg-p2 p-4"
    >
      <div className="flex items-start gap-2.5">
        <KeyIcon />
        <div className="min-w-0">
          <h3 className="font-ui text-xs font-semibold uppercase tracking-wider text-p10">
            {t("gmailGuide_title")}
          </h3>
          <p className="mt-1 font-body text-xs leading-relaxed text-p8">{t("gmailGuide_intro")}</p>
        </div>
      </div>

      <p className="border-amber/40 bg-amber/10 mt-3 rounded-chip border px-3 py-2 font-body text-xs leading-relaxed text-p9">
        {t("gmailGuide_prereq")}
      </p>

      <ol className="mt-3 space-y-2">
        {steps.map((text, i) => (
          <li
            key={i}
            className="flex items-start gap-2.5 font-body text-xs leading-relaxed text-p9"
          >
            <span className="mt-px inline-flex h-5 w-5 flex-none items-center justify-center rounded-avatar bg-p9 font-mono text-[10px] text-white">
              {i + 1}
            </span>
            <span className="min-w-0">{text}</span>
          </li>
        ))}
      </ol>

      <a
        href={APP_PASSWORDS_URL}
        rel="noreferrer"
        onClick={(e) => {
          e.preventDefault();
          void openExternalUrl(APP_PASSWORDS_URL);
        }}
        className="mt-3 inline-flex items-center gap-2 rounded-chip bg-p10 px-3 py-2 font-ui text-[11px] font-semibold uppercase tracking-wider text-white"
      >
        {t("gmailGuide_open")}
        <ExternalIcon />
      </a>

      <details className="mt-3 border-t border-divider pt-2">
        <summary className="cursor-pointer font-ui text-[10px] uppercase tracking-wider text-p8">
          {t("gmailGuide_trouble_summary")}
        </summary>
        <div className="mt-2 space-y-2 font-body text-xs leading-relaxed text-p8">
          <p>
            <span className="font-semibold text-p9">{t("gmailGuide_trouble_q")}</span>{" "}
            {t("gmailGuide_trouble_a")}
          </p>
          <p>
            <span className="font-semibold text-p9">{t("gmailGuide_wrong_q")}</span>{" "}
            {t("gmailGuide_wrong_a")}
          </p>
        </div>
      </details>
    </section>
  );
}

function KeyIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      fill="none"
      className="mt-0.5 h-4 w-4 flex-none text-amber"
    >
      <circle cx="8" cy="12" r="4.5" stroke="currentColor" strokeWidth="1.7" />
      <path
        d="M12 12h9M18 12v3M15 12v2"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
    </svg>
  );
}

function ExternalIcon() {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" className="h-3 w-3">
      <path
        d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6M15 3h6v6M10 14 21 3"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
