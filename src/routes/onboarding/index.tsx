// First-run onboarding wizard (T046). Standalone full-screen route — not a child
// of AppShell. Four steps: Welcome → Add Account → Knowledge Depth → Ready.
// Step state is component-local (07 §5: ephemeral UI, not Zustand, not URL).
// Focus moves to the new step heading on each transition (dev/11 §3).
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/cn";
import { useAccounts } from "@/ipc/queries/accounts";
import AddAccountWizard from "@/routes/settings/accounts/AddAccountWizard";
import BrandMark from "@/components/brand/BrandMark";

// ── Types ─────────────────────────────────────────────────────────────────────

type Step = 1 | 2 | 3 | 4;

type DepthChoice = "3m" | "1y" | "all";

interface DepthOption {
  value: DepthChoice;
  labelKey: string;
  descKey: string;
}

const DEPTH_OPTIONS: DepthOption[] = [
  { value: "3m", labelKey: "onboarding_depth_3m", descKey: "onboarding_depth_3m_desc" },
  { value: "1y", labelKey: "onboarding_depth_1y", descKey: "onboarding_depth_1y_desc" },
  { value: "all", labelKey: "onboarding_depth_all", descKey: "onboarding_depth_all_desc" },
];

const STEP_LABELS: Record<Step, string> = {
  1: "step_welcome",
  2: "step_add_account",
  3: "step_knowledge",
  4: "step_ready",
};

// ── Progress dots ─────────────────────────────────────────────────────────────

function StepDots({ current }: { current: Step }) {
  return (
    <div className="flex items-center gap-2" aria-hidden>
      {([1, 2, 3, 4] as Step[]).map((s) => (
        <span
          key={s}
          className={cn(
            "h-2 w-2 rounded-avatar transition-colors",
            s === current ? "bg-p9" : s < current ? "bg-p7" : "bg-p5",
          )}
        />
      ))}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export default function Onboarding() {
  const { t } = useTranslation("settings");
  const navigate = useNavigate();

  const [step, setStep] = useState<Step>(1);
  const [depthChoice, setDepthChoice] = useState<DepthChoice>("1y");
  // Whether the user has successfully added an account in Step 2.
  // Detected by watching the accounts query rather than wizard callback,
  // because AddAccountWizard.onClose fires for both cancel and success.
  const [accountAdded, setAccountAdded] = useState(false);
  // Show the embedded AddAccountWizard overlay.
  const [showAddWizard, setShowAddWizard] = useState(false);

  // Watch for a new account to be created during this session.
  // We capture the count when the page first loads so we only advance after
  // the user actually adds one (not just because mock data returns a demo account).
  const { data: accounts } = useAccounts();
  const initialCountRef = useRef<number | null>(null);
  if (initialCountRef.current === null && accounts !== undefined) {
    initialCountRef.current = accounts.length;
  }

  const headingRef = useRef<HTMLHeadingElement>(null);

  // Move focus to heading on step change (dev/11 §3).
  useEffect(() => {
    headingRef.current?.focus();
  }, [step]);

  const advanceTo = (next: Step) => setStep(next);

  // Detect when a new account is added relative to the initial count.
  useEffect(() => {
    const initial = initialCountRef.current;
    if (!accountAdded && initial !== null && (accounts?.length ?? 0) > initial) {
      setAccountAdded(true);
      setShowAddWizard(false);
    }
  }, [accounts, accountAdded]);

  const handleFinish = () => {
    // Knowledge depth is stored per-account in AddAccountWizard's KnowledgeDepthStep.
    // Here we persist the onboarding-level choice as an app-level preference.
    // The actual IPC command (set_app_setting) is not yet in the command surface;
    // we record the selection in memory and navigate to the dashboard.
    void navigate("/", { replace: true });
  };

  return (
    <main className="flex min-h-screen flex-col items-center justify-center bg-parchment px-4 py-10">
      {/* Embedded add-account overlay (Step 2 only). onClose fires for both
          cancel and success; account detection is handled by the useEffect
          above which watches the accounts query. */}
      {showAddWizard && (
        <AddAccountWizard
          onClose={() => {
            setShowAddWizard(false);
          }}
        />
      )}

      <div className="w-full max-w-[480px]">
        {/* Brand mark */}
        <div className="mb-6 flex flex-col items-center gap-2">
          <BrandMark size={44} />
          <p className="section-label text-p8">SeekerMail</p>
        </div>

        {/* Step card */}
        <div className="rounded-card bg-surface shadow-card">
          {/* Card header: step dots + step label */}
          <div className="flex items-center justify-between border-b border-divider px-6 py-4">
            <StepDots current={step} />
            <span className="font-mono text-xs text-p8">
              {t(STEP_LABELS[step])} · {step}/4
            </span>
          </div>

          {/* Card body */}
          <div className="px-6 py-8">
            {step === 1 && (
              <StepWelcome headingRef={headingRef} onNext={() => advanceTo(2)} t={t} />
            )}
            {step === 2 && (
              <StepAddAccount
                headingRef={headingRef}
                accountAdded={accountAdded}
                onOpenWizard={() => setShowAddWizard(true)}
                onNext={() => advanceTo(3)}
                t={t}
              />
            )}
            {step === 3 && (
              <StepKnowledgeDepth
                headingRef={headingRef}
                choice={depthChoice}
                onChoose={setDepthChoice}
                onNext={() => advanceTo(4)}
                t={t}
              />
            )}
            {step === 4 && <StepReady headingRef={headingRef} onFinish={handleFinish} t={t} />}
          </div>
        </div>

        {/* Local-first reassurance */}
        <p className="mt-6 text-center font-body text-xs text-p8">
          Your mail stays on this device. Nothing is uploaded to SeekerMail servers.
        </p>
      </div>
    </main>
  );
}

// ── Step sub-components ───────────────────────────────────────────────────────

// Translation function type — narrow enough for what we need.
type TFn = (key: string) => string;

function CtaButton({
  onClick,
  children,
  secondary = false,
}: {
  onClick: () => void;
  children: React.ReactNode;
  secondary?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-chip px-5 py-2.5 font-ui text-sm font-medium uppercase tracking-wide transition-colors",
        secondary
          ? "text-p8 hover:text-p9"
          : "bg-p9 text-white hover:bg-p10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-p9",
      )}
    >
      {children}
    </button>
  );
}

// Step 1 — Welcome
function StepWelcome({
  headingRef,
  onNext,
  t,
}: {
  headingRef: React.RefObject<HTMLHeadingElement>;
  onNext: () => void;
  t: TFn;
}) {
  return (
    <div className="flex flex-col items-center gap-6 text-center">
      {/* Decorative mark */}
      <div className="flex h-16 w-16 items-center justify-center rounded-avatar bg-p4" aria-hidden>
        <span className="font-display text-2xl italic text-p9">S</span>
      </div>

      <div>
        <h1
          ref={headingRef}
          tabIndex={-1}
          className="font-display text-3xl italic text-p10 outline-none"
        >
          {t("onboarding_welcome_title")}
        </h1>
        <p className="mt-3 font-body text-sm leading-relaxed text-p8">
          {t("onboarding_welcome_subtitle")}
        </p>
      </div>

      <div className="flex flex-col items-center gap-2">
        <CtaButton onClick={onNext}>{t("btn_get_started")}</CtaButton>
      </div>
    </div>
  );
}

// Step 2 — Add Account
function StepAddAccount({
  headingRef,
  accountAdded,
  onOpenWizard,
  onNext,
  t,
}: {
  headingRef: React.RefObject<HTMLHeadingElement>;
  accountAdded: boolean;
  onOpenWizard: () => void;
  onNext: () => void;
  t: TFn;
}) {
  return (
    <div className="flex flex-col gap-5">
      <div>
        <h1
          ref={headingRef}
          tabIndex={-1}
          className="font-display text-2xl italic text-p10 outline-none"
        >
          {t("onboarding_add_account_title")}
        </h1>
        <p className="mt-2 font-body text-sm leading-relaxed text-p8">
          {t("onboarding_add_account_subtitle")}
        </p>
      </div>

      {accountAdded ? (
        <div className="rounded-card border border-divider bg-p2 px-4 py-3">
          <p className="font-body text-sm text-green">Account added successfully.</p>
        </div>
      ) : (
        <div className="rounded-card border border-divider bg-p2 px-4 py-3 text-center">
          <p className="font-body text-xs text-p8">No accounts connected yet.</p>
        </div>
      )}

      <div className="flex flex-col gap-2">
        {!accountAdded && (
          <CtaButton onClick={onOpenWizard}>{t("onboarding_add_account")}</CtaButton>
        )}
        {accountAdded && <CtaButton onClick={onNext}>{t("btn_continue")}</CtaButton>}
      </div>
    </div>
  );
}

// Step 3 — Knowledge Depth
function StepKnowledgeDepth({
  headingRef,
  choice,
  onChoose,
  onNext,
  t,
}: {
  headingRef: React.RefObject<HTMLHeadingElement>;
  choice: DepthChoice;
  onChoose: (v: DepthChoice) => void;
  onNext: () => void;
  t: TFn;
}) {
  return (
    <div className="flex flex-col gap-5">
      <div>
        <h1
          ref={headingRef}
          tabIndex={-1}
          className="font-display text-2xl italic text-p10 outline-none"
        >
          {t("onboarding_knowledge_depth_title")}
        </h1>
        <p className="mt-2 font-body text-sm leading-relaxed text-p8">
          {t("onboarding_knowledge_depth_subtitle")}
        </p>
      </div>

      <ul
        className="flex flex-col gap-2"
        role="radiogroup"
        aria-label={t("onboarding_knowledge_depth_title")}
      >
        {DEPTH_OPTIONS.map((opt) => {
          const selected = choice === opt.value;
          return (
            <li key={opt.value}>
              <label
                className={cn(
                  "flex cursor-pointer items-start gap-3 rounded-card border px-4 py-3 transition-colors",
                  selected ? "border-slate bg-p2" : "border-divider bg-surface hover:bg-p4",
                )}
              >
                <input
                  type="radio"
                  name="knowledge-depth-onboarding"
                  value={opt.value}
                  checked={selected}
                  onChange={() => onChoose(opt.value)}
                  className="mt-0.5 accent-p9"
                  aria-required
                />
                <span className="flex flex-col gap-0.5">
                  <span className="font-ui text-sm font-medium text-p9">{t(opt.labelKey)}</span>
                  <span className="font-body text-xs text-p8">{t(opt.descKey)}</span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>

      <CtaButton onClick={onNext}>{t("btn_continue")}</CtaButton>
    </div>
  );
}

// Step 4 — Ready
function StepReady({
  headingRef,
  onFinish,
  t,
}: {
  headingRef: React.RefObject<HTMLHeadingElement>;
  onFinish: () => void;
  t: TFn;
}) {
  return (
    <div className="flex flex-col items-center gap-6 text-center">
      <div
        className="bg-green/15 flex h-16 w-16 items-center justify-center rounded-avatar"
        aria-hidden
      >
        <span className="font-mono text-2xl text-green">✓</span>
      </div>

      <div>
        <h1
          ref={headingRef}
          tabIndex={-1}
          className="font-display text-3xl italic text-p10 outline-none"
        >
          {t("onboarding_ready_title")}
        </h1>
        <p className="mt-3 font-body text-sm leading-relaxed text-p8">
          {t("onboarding_ready_subtitle")}
        </p>
      </div>

      <CtaButton onClick={onFinish}>{t("btn_open_seekermail")}</CtaButton>
    </div>
  );
}
