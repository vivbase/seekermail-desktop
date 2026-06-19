// Compose route (T044 + T045, F_G4). Assembles the full compose view:
// toolbar → recipient fields → subject → body editor → attachments → footer.
// Reads reply/forward seed from react-router location.state and populates the
// compose store. Wires draft autosave via useDraftAutosave.

import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useTranslation } from "react-i18next";

import type { MailDetail, SendMailParams } from "@shared/bindings";
import { useCompose } from "@/stores/compose";
import { useSendMail } from "@/hooks/useSendMail";
import { useDraftAutosave } from "@/hooks/useDraftAutosave";
import { useDeleteDraft, type AiComposeSeed } from "@/ipc/queries/drafts";
import { parseRecipients, validateCompose } from "@/lib/composeValidation";
import { buildReplySeed, buildReplyAllSeed, buildForwardSeed } from "@/lib/quoteBuilder";
import { isHtmlBlank, plainTextToHtml } from "@/lib/richText";

import { ComposeToolbar, type ComposeMode } from "@/components/compose/ComposeToolbar";
import { RecipientInput } from "@/components/compose/RecipientInput";
import { ComposeEditor } from "@/components/compose/ComposeEditor";
import { AttachmentBar } from "@/components/compose/AttachmentBar";
import { ComposeFooter, type ValidationWarning } from "@/components/compose/ComposeFooter";

// ── Location state shape ─────────────────────────────────────────────────────

/**
 * Shape of react-router location.state when navigating to /compose from a mail
 * view. All fields are optional; their absence implies "new" mode.
 */
interface ComposeSeedState {
  mode?: ComposeMode;
  mail?: MailDetail;
  ownEmail?: string;
  /** AI-generated draft seed (T078 E1 success, T081 "Open in Compose"). */
  aiSeed?: AiComposeSeed;
}

// ── Route component ──────────────────────────────────────────────────────────

export default function Compose() {
  const { t } = useTranslation("compose");
  const { t: tNav } = useTranslation("nav");
  const navigate = useNavigate();
  const location = useLocation();

  // ── Compose store ──────────────────────────────────────────────────────

  const open = useCompose((s) => s.open);
  const reset = useCompose((s) => s.reset);
  const accountId = useCompose((s) => s.accountId);
  const to = useCompose((s) => s.to);
  const cc = useCompose((s) => s.cc);
  const bcc = useCompose((s) => s.bcc);
  const subject = useCompose((s) => s.subject);
  const body = useCompose((s) => s.body);
  const bodyHtml = useCompose((s) => s.bodyHtml);
  const inReplyTo = useCompose((s) => s.inReplyTo);
  const draftId = useCompose((s) => s.draftId);
  const ccVisible = useCompose((s) => s.ccVisible);
  const update = useCompose((s) => s.update);

  // ── Mode + seeding ─────────────────────────────────────────────────────

  const seedAppliedRef = useRef(false);
  const state = (location.state ?? {}) as ComposeSeedState;
  const mode: ComposeMode = state.mode ?? "new";

  useEffect(() => {
    if (seedAppliedRef.current) return;
    seedAppliedRef.current = true;

    if (state.aiSeed) {
      // AI draft seed (E1/E6): body already generated, no quote block appended.
      open({
        accountId: state.aiSeed.accountId,
        to: state.aiSeed.to,
        subject: state.aiSeed.subject,
        body: state.aiSeed.body,
        bodyHtml: plainTextToHtml(state.aiSeed.body),
        inReplyTo: state.aiSeed.inReplyTo,
        aiDraftId: state.aiSeed.aiDraftId,
      });
      return;
    }
    if (state.mail) {
      let seed: ReturnType<typeof buildReplySeed>;
      if (mode === "reply") {
        seed = buildReplySeed(state.mail);
      } else if (mode === "reply-all") {
        seed = buildReplyAllSeed(state.mail, state.ownEmail ?? "");
      } else if (mode === "forward") {
        seed = buildForwardSeed(state.mail);
      } else {
        open();
        return;
      }
      open({
        // From-account defaults to the account that received the original mail
        // so the From selector is pre-filled for reply / reply-all / forward.
        accountId: state.mail.accountId,
        to: seed.to,
        cc: seed.cc,
        subject: seed.subject,
        body: seed.body,
        bodyHtml: plainTextToHtml(seed.body),
        inReplyTo: seed.inReplyTo,
      });
    } else {
      open();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Reset the store on unmount to avoid stale data leaking into the next compose.
  useEffect(() => {
    return () => {
      reset();
    };
  }, [reset]);

  // ── Send mail controller ───────────────────────────────────────────────

  const sender = useSendMail();
  const { mutate: deleteDraft } = useDeleteDraft();

  // ── Draft autosave ─────────────────────────────────────────────────────

  const autosave = useDraftAutosave();

  // ── Attachment count (kept in route state, passed to validation) ───────

  const [attachmentCount, setAttachmentCount] = useState(0);

  // ── Pre-send validation ────────────────────────────────────────────────

  const [activeWarnings, setActiveWarnings] = useState<ValidationWarning[]>([]);

  const validationResult = validateCompose({
    accountId,
    to,
    subject,
    body,
    attachmentCount,
  });

  const blockingErrorMessages = validationResult.errors.map((e) => e.message);

  // ── Params builder ─────────────────────────────────────────────────────

  function buildParams(): SendMailParams {
    return {
      accountId: accountId ?? "",
      to: parseRecipients(to),
      cc: parseRecipients(cc),
      bcc: parseRecipients(bcc),
      subject,
      bodyText: body,
      bodyHtml: isHtmlBlank(bodyHtml) ? null : bodyHtml,
      inReplyTo: inReplyTo,
      references: null,
      draftId: draftId,
    };
  }

  // ── Send entry point (Ctrl+Enter from the body editor) ─────────────────

  function handleSendIntent() {
    if (!validationResult.ok) {
      // Hard block: do nothing. The footer Send button is already disabled and
      // shows error context.
      return;
    }
    if (validationResult.warnings.length > 0) {
      // Surface soft warnings so the user can acknowledge then send.
      setActiveWarnings(validationResult.warnings.map((w) => ({ message: w.message })));
      return;
    }
    // No blockers, no warnings: send immediately.
    setActiveWarnings([]);
    void sender.send(buildParams()).then(() => {
      if (draftId) deleteDraft(draftId);
      reset();
    });
  }

  // ── Close / discard navigation ─────────────────────────────────────────

  const [discardRequested, setDiscardRequested] = useState(false);

  function handleClose() {
    const hasContent = to.trim() || subject.trim() || body.trim();
    if (!hasContent) {
      // Nothing to lose: skip the confirmation dialog. Fixed parent: compose
      // returns to Inbox (root CLAUDE.md back-button rule), never browser history.
      reset();
      void navigate("/all-mail");
    } else {
      // Delegate to ComposeFooter's confirmation dialog.
      setDiscardRequested(true);
    }
  }

  // ── Page title ─────────────────────────────────────────────────────────

  const titleKey =
    mode === "reply"
      ? "title_reply"
      : mode === "reply-all"
        ? "title_reply_all"
        : mode === "forward"
          ? "title_forward"
          : "title";

  // ── Render ─────────────────────────────────────────────────────────────

  return (
    <section className="flex h-full flex-col bg-surface">
      {/* Page header */}
      <div className="border-b border-divider px-5 py-4">
        <button type="button" className="pg-back" onClick={handleClose}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
            <path
              d="M9 2L4 7l5 5"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          {tNav("back_to_inbox")}
        </button>
        <p className="section-label mb-1">Compose</p>
        <h1 className="font-display text-2xl italic text-p10">{t(titleKey)}</h1>
      </div>

      {/* Scrollable form body */}
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
        {/* Toolbar: From selector + mode badge + Cc toggle + close */}
        <ComposeToolbar mode={mode} onClose={handleClose} />

        {/* Recipient fields */}
        <RecipientInput
          label={t("to_label")}
          value={to}
          onChange={(v) => update({ to: v })}
          placeholder={t("to_placeholder")}
          autoFocus={mode === "new"}
        />

        {ccVisible && (
          <>
            <RecipientInput
              label={t("cc_label")}
              value={cc}
              onChange={(v) => update({ cc: v })}
              placeholder={t("cc_placeholder")}
            />
            <RecipientInput
              label={t("bcc_label")}
              value={bcc}
              onChange={(v) => update({ bcc: v })}
              placeholder={t("bcc_placeholder")}
            />
          </>
        )}

        {/* Subject */}
        <div className="flex items-baseline gap-3 border-b border-divider px-5 py-2.5">
          <label
            htmlFor="compose-subject"
            className="w-16 shrink-0 font-ui text-[10px] font-semibold uppercase tracking-widest text-p8"
          >
            {t("subject_label")}
          </label>
          <input
            id="compose-subject"
            type="text"
            value={subject}
            onChange={(e) => update({ subject: e.target.value })}
            placeholder={t("subject_placeholder")}
            autoFocus={mode !== "new"}
            className="min-w-0 flex-1 bg-transparent font-body text-sm text-p10 placeholder:text-p7 focus:outline-none"
          />
        </div>

        {/* Body editor */}
        <ComposeEditor onSend={handleSendIntent} />

        {/* Attachment staging bar */}
        <AttachmentBar onCountChange={setAttachmentCount} />
      </div>

      {/* Footer: send, discard, warnings, undo banner */}
      <ComposeFooter
        sender={sender}
        autosave={autosave}
        blockingErrors={blockingErrorMessages}
        warnings={activeWarnings}
        buildParams={buildParams}
        onClearWarnings={() => setActiveWarnings([])}
        onSendClick={handleSendIntent}
        discardRequested={discardRequested}
        onDiscardRequestHandled={() => setDiscardRequested(false)}
      />
    </section>
  );
}
