// Add-account wizard (T017). Five steps held in component state (no routing):
// protocol → credentials → connection test → knowledge depth → confirm. All data
// flows through the IPC hooks; no component-level `invoke`.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  CreateAccountParams,
  Provider,
  SamplingResult,
  VerifyConnectionResult,
} from "@shared/bindings";

import {
  useBeginOAuth,
  useCreateAccount,
  useSampleMailbox,
  useSetKnowledgeDepth,
  useVerifyConnection,
} from "@/ipc/queries/accounts";
import KnowledgeDepthStep from "./KnowledgeDepthStep";

interface AddAccountWizardProps {
  onClose: () => void;
}

type Step = 1 | 2 | 3 | 4 | 5;

const COLOR_TOKENS = ["slate", "terra", "sage"] as const;

export default function AddAccountWizard({ onClose }: AddAccountWizardProps) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>(1);
  const [provider, setProvider] = useState<Provider>("imap");
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [colorToken, setColorToken] = useState<string>("slate");
  const [badgeLabel, setBadgeLabel] = useState("W");
  const [imapHost, setImapHost] = useState("");
  const [imapPort, setImapPort] = useState("993");
  const [smtpHost, setSmtpHost] = useState("");
  const [smtpPort, setSmtpPort] = useState("587");

  const [accountId, setAccountId] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<VerifyConnectionResult | null>(null);
  const [sampling, setSampling] = useState<SamplingResult | null>(null);
  const [depth, setDepth] = useState<number | null>(12);

  const verify = useVerifyConnection();
  const create = useCreateAccount();
  const beginOAuth = useBeginOAuth();
  const sample = useSampleMailbox();
  const setKnowledgeDepth = useSetKnowledgeDepth();

  const isOAuth = provider === "gmail" || provider === "outlook";

  const num = (s: string): number | null => (s.trim() === "" ? null : Number(s));

  const buildParams = (): CreateAccountParams => ({
    email,
    displayName: displayName || email,
    provider,
    imapHost: imapHost || null,
    imapPort: num(imapPort),
    smtpHost: smtpHost || null,
    smtpPort: num(smtpPort),
    colorToken,
    badgeLabel,
    roleType: null,
    roleDescription: null,
    authLevel: null,
    password: isOAuth ? null : password || null,
  });

  // Sample the mailbox once we have a created account and reach the depth step.
  useEffect(() => {
    if (step === 4 && accountId && !sampling && !sample.isPending) {
      sample.mutate(accountId, { onSuccess: setSampling });
    }
  }, [step, accountId, sampling, sample]);

  const runTest = () => {
    setTestResult(null);
    verify.mutate(
      {
        email,
        provider,
        password: password || null,
        imapHost: imapHost || null,
        imapPort: num(imapPort),
        imapTls: null,
        smtpHost: smtpHost || null,
        smtpPort: num(smtpPort),
        smtpTls: null,
      },
      { onSuccess: setTestResult },
    );
  };

  const createThenAdvance = () => {
    create.mutate(buildParams(), {
      onSuccess: (acc) => {
        setAccountId(acc.id);
        setStep(4);
        if (isOAuth) beginOAuth.mutate({ provider, accountId: acc.id });
      },
    });
  };

  const finish = () => {
    if (accountId) {
      setKnowledgeDepth.mutate({ accountId, months: depth }, { onSuccess: onClose });
    } else {
      onClose();
    }
  };

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-40 flex items-center justify-center p-4"
      role="presentation"
    >
      <div
        className="flex max-h-[90vh] w-full max-w-lg flex-col overflow-hidden rounded-card bg-surface shadow-card"
        role="dialog"
        aria-modal="true"
      >
        <header className="flex items-center justify-between border-b border-divider px-5 py-4">
          <h2 className="font-display text-xl italic text-p10">{t("wizard_title")}</h2>
          <span className="font-mono text-xs text-p8">{step}/5</span>
        </header>

        <div className="flex-1 overflow-y-auto p-5">
          {step === 1 && (
            <div className="space-y-2">
              <p className="font-ui text-[10px] uppercase tracking-wider text-p8">
                {t("wizard_step_protocol")}
              </p>
              {(
                [
                  ["imap", "wizard_protocol_imap"],
                  ["gmail", "wizard_protocol_oauth"],
                ] as const
              ).map(([value, key]) => (
                <button
                  key={value}
                  type="button"
                  onClick={() => setProvider(value)}
                  className={`block w-full rounded-card border px-4 py-3 text-start font-body text-sm ${
                    provider === value ? "border-slate bg-p2 text-p10" : "border-divider text-p9"
                  }`}
                >
                  {t(key)}
                </button>
              ))}
              <button
                type="button"
                disabled
                className="block w-full cursor-not-allowed rounded-card border border-divider px-4 py-3 text-start font-body text-sm text-p7 opacity-60"
              >
                {t("wizard_protocol_exchange")}
              </button>
            </div>
          )}

          {step === 2 && (
            <div className="space-y-3">
              <Field label={t("wizard_email")} value={email} onChange={setEmail} type="email" />
              <Field
                label={t("wizard_display_name")}
                value={displayName}
                onChange={setDisplayName}
              />
              {!isOAuth && (
                <Field
                  label={t("wizard_password")}
                  value={password}
                  onChange={setPassword}
                  type="password"
                />
              )}
              <div className="grid grid-cols-2 gap-3">
                <Field label={t("wizard_imap_host")} value={imapHost} onChange={setImapHost} />
                <Field label={t("wizard_imap_port")} value={imapPort} onChange={setImapPort} />
                <Field label={t("wizard_smtp_host")} value={smtpHost} onChange={setSmtpHost} />
                <Field label={t("wizard_smtp_port")} value={smtpPort} onChange={setSmtpPort} />
              </div>
              <div>
                <p className="font-ui text-[10px] uppercase tracking-wider text-p8">
                  {t("wizard_color")}
                </p>
                <div className="mt-1 flex gap-2">
                  {COLOR_TOKENS.map((tok) => (
                    <button
                      key={tok}
                      type="button"
                      aria-label={tok}
                      onClick={() => setColorToken(tok)}
                      className={`h-7 w-7 rounded-avatar bg-${tok} ${
                        colorToken === tok ? "ring-2 ring-p9" : ""
                      }`}
                    />
                  ))}
                  <input
                    aria-label={t("wizard_badge")}
                    value={badgeLabel}
                    maxLength={1}
                    onChange={(e) => setBadgeLabel(e.target.value)}
                    className="h-7 w-10 rounded-chip border border-divider text-center font-mono text-sm"
                  />
                </div>
              </div>
            </div>
          )}

          {step === 3 && (
            <div className="space-y-3">
              <button
                type="button"
                onClick={runTest}
                disabled={verify.isPending}
                className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white"
              >
                {verify.isPending ? t("wizard_test_running") : t("wizard_test_run")}
              </button>
              {testResult && (
                <ul className="space-y-1 font-body text-sm">
                  <li className={testResult.imapOk ? "text-green" : "text-red"}>
                    {testResult.imapOk ? t("wizard_test_imap_ok") : t("wizard_test_failed")}
                  </li>
                  <li className={testResult.smtpOk ? "text-green" : "text-red"}>
                    {testResult.smtpOk ? t("wizard_test_smtp_ok") : t("wizard_test_failed")}
                  </li>
                  {testResult.errorMessage && (
                    <li className="font-mono text-xs text-p8">{testResult.errorMessage}</li>
                  )}
                </ul>
              )}
            </div>
          )}

          {step === 4 && (
            <KnowledgeDepthStep sampling={sampling} selected={depth} onSelect={setDepth} />
          )}

          {step === 5 && (
            <div className="font-body text-sm text-p9">
              <p>{email}</p>
              <p className="text-p8">
                {depth === null ? t("depth_all") : t("depth_months", { months: depth })}
              </p>
            </div>
          )}
        </div>

        <footer className="flex items-center justify-between border-t border-divider px-5 py-4">
          <button
            type="button"
            onClick={step === 1 ? onClose : () => setStep((s) => (s - 1) as Step)}
            className="font-ui text-xs uppercase tracking-wider text-p8"
          >
            {step === 1 ? t("wizard_cancel") : t("wizard_back")}
          </button>
          <WizardNext
            step={step}
            isOAuth={isOAuth}
            canAdvanceTest={!!testResult?.imapOk}
            onProtocol={() => setStep(2)}
            onCredentials={() => (isOAuth ? createThenAdvance() : setStep(3))}
            onTest={createThenAdvance}
            onDepth={() => setStep(5)}
            onFinish={finish}
          />
        </footer>
      </div>
    </div>
  );
}

function WizardNext(props: {
  step: Step;
  isOAuth: boolean;
  canAdvanceTest: boolean;
  onProtocol: () => void;
  onCredentials: () => void;
  onTest: () => void;
  onDepth: () => void;
  onFinish: () => void;
}) {
  const { t } = useTranslation();
  const cls =
    "rounded-chip bg-p9 px-4 py-1.5 font-ui text-xs uppercase tracking-wider text-white disabled:opacity-50";
  switch (props.step) {
    case 1:
      return (
        <button type="button" className={cls} onClick={props.onProtocol}>
          {t("wizard_next")}
        </button>
      );
    case 2:
      return (
        <button type="button" className={cls} onClick={props.onCredentials}>
          {t("wizard_next")}
        </button>
      );
    case 3:
      return (
        <button
          type="button"
          className={cls}
          disabled={!props.canAdvanceTest}
          onClick={props.onTest}
        >
          {t("wizard_next")}
        </button>
      );
    case 4:
      return (
        <button type="button" className={cls} onClick={props.onDepth}>
          {t("wizard_next")}
        </button>
      );
    case 5:
      return (
        <button type="button" className={cls} onClick={props.onFinish}>
          {t("wizard_create")}
        </button>
      );
  }
}

function Field(props: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  type?: string;
}) {
  return (
    <label className="block">
      <span className="font-ui text-[10px] uppercase tracking-wider text-p8">{props.label}</span>
      <input
        type={props.type ?? "text"}
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
        className="mt-1 w-full rounded-chip border border-divider bg-surface px-3 py-2 font-body text-sm text-p9"
      />
    </label>
  );
}
