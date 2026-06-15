// Plain-text body editor for v0.x (T044, F_G4 §4.4). Tiptap rich text is planned
// for v0.5+ once the dependency is locked; for now this is a large comfortable
// textarea with autosize via a hidden mirror element. Keyboard shortcut
// Ctrl/Cmd+Enter triggers send via the onSend callback.

import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useCompose } from "@/stores/compose";

// ── Types ────────────────────────────────────────────────────────────────────

interface ComposeEditorProps {
  /** Called when the user presses Ctrl/Cmd+Enter. */
  onSend: () => void;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/** Resize the textarea to fit its content using a hidden-mirror approach. */
function autosizeTextarea(textarea: HTMLTextAreaElement, mirror: HTMLDivElement) {
  // Sync mirror content and measure.
  mirror.textContent = textarea.value + "\n";
  textarea.style.height = `${mirror.scrollHeight}px`;
}

// ── Component ────────────────────────────────────────────────────────────────

export function ComposeEditor({ onSend }: ComposeEditorProps) {
  const { t } = useTranslation("compose");

  const body = useCompose((s) => s.body);
  const update = useCompose((s) => s.update);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);

  // Autosize whenever content changes.
  useEffect(() => {
    if (textareaRef.current && mirrorRef.current) {
      autosizeTextarea(textareaRef.current, mirrorRef.current);
    }
  }, [body]);

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    // Ctrl+Enter or Cmd+Enter → trigger send.
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      onSend();
    }
  }

  return (
    <div className="relative flex-1 px-5 py-4">
      {/* Hidden mirror for autosize measurement */}
      <div
        ref={mirrorRef}
        aria-hidden="true"
        className="invisible absolute inset-x-5 top-4 whitespace-pre-wrap break-words font-body text-sm"
        style={{ pointerEvents: "none" }}
      />

      <textarea
        ref={textareaRef}
        id="compose-body"
        aria-label="Message body"
        value={body}
        onChange={(e) => update({ body: e.target.value })}
        onKeyDown={handleKeyDown}
        placeholder={t("body_placeholder")}
        spellCheck
        rows={10}
        className={[
          "w-full resize-none overflow-hidden bg-transparent",
          "font-body text-sm leading-relaxed text-p10",
          "placeholder:text-p7 focus:outline-none",
        ].join(" ")}
      />
    </div>
  );
}
