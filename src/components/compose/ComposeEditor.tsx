// Rich-text body editor for compose (T044, F_G4 §4.4). A contentEditable surface
// paired with ComposeFormatBar provides the Gmail-equivalent formatting baseline
// (bold/italic/underline, font size, colour, lists, links, …). The editor keeps
// two representations in the compose store in lock-step: `bodyHtml` (the rendered
// HTML, sent as the text/html MIME part) and `body` (a plain-text mirror derived
// from innerText, used for validation, autosave's bodyText, and quote seeds).
// Ctrl/Cmd+Enter triggers send via onSend.

import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useCompose } from "@/stores/compose";
import { htmlToPlainText, isHtmlBlank } from "@/lib/richText";
import { ComposeFormatBar } from "./ComposeFormatBar";

// ── Types ────────────────────────────────────────────────────────────────────

interface ComposeEditorProps {
  /** Called when the user presses Ctrl/Cmd+Enter. */
  onSend: () => void;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/**
 * Derive the plain-text mirror. The rendered webview exposes `innerText`
 * (layout-aware, preserves line breaks); the htmlToPlainText fallback covers
 * environments where it is absent (e.g. jsdom under test).
 */
function getPlainText(el: HTMLDivElement): string {
  return el.innerText ?? htmlToPlainText(el.innerHTML);
}

// ── Component ────────────────────────────────────────────────────────────────

export function ComposeEditor({ onSend }: ComposeEditorProps) {
  const { t } = useTranslation("compose");

  const bodyHtml = useCompose((s) => s.bodyHtml);
  const update = useCompose((s) => s.update);

  const editorRef = useRef<HTMLDivElement>(null);
  /** Last HTML this editor emitted, so external store updates (seed / AI
   *  regenerate / reset) re-render the DOM while typing never clobbers the caret. */
  const lastHtmlRef = useRef<string>("");

  // Sync store → DOM only when the change originated outside the editor.
  useEffect(() => {
    const el = editorRef.current;
    if (!el) return;
    if (bodyHtml !== lastHtmlRef.current && bodyHtml !== el.innerHTML) {
      el.innerHTML = bodyHtml;
      lastHtmlRef.current = bodyHtml;
    }
  }, [bodyHtml]);

  function handleInput() {
    const el = editorRef.current;
    if (!el) return;
    const html = el.innerHTML;
    lastHtmlRef.current = html;
    update({ bodyHtml: html, body: getPlainText(el) });
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      onSend();
    }
  }

  const showPlaceholder = isHtmlBlank(bodyHtml);

  return (
    <div className="flex flex-1 flex-col">
      <ComposeFormatBar editorRef={editorRef} />

      <div className="relative flex-1 px-5 py-4">
        {showPlaceholder && (
          <p
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-5 top-4 font-body text-sm text-p7"
          >
            {t("body_placeholder")}
          </p>
        )}

        <div
          ref={editorRef}
          id="compose-body"
          role="textbox"
          aria-multiline="true"
          aria-label="Message body"
          contentEditable
          suppressContentEditableWarning
          spellCheck
          onInput={handleInput}
          onKeyDown={handleKeyDown}
          className={[
            "compose-editor min-h-[12rem] w-full bg-transparent",
            "font-body text-sm leading-relaxed text-p10",
            "focus:outline-none",
          ].join(" ")}
        />
      </div>
    </div>
  );
}
