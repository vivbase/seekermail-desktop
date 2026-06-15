// Save-search dialog (T035). A small modal that names and persists the current
// query via useSaveSearch. Mode is taken from useUi.searchMode. Follows the
// ConfirmDialog pattern — no external UI library, token-styled.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useSaveSearch } from "@/ipc/queries/search";
import { useUi } from "@/stores/ui";
import { cn } from "@/lib/cn";

export interface SaveSearchDialogProps {
  open: boolean;
  query: string;
  onClose: () => void;
}

export function SaveSearchDialog({ open, query, onClose }: SaveSearchDialogProps) {
  const { t } = useTranslation("search");
  const searchMode = useUi((s) => s.searchMode);
  const saveSearch = useSaveSearch();

  const [name, setName] = useState(() => query.slice(0, 40));
  const [inlineError, setInlineError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset state when the dialog opens.
  useEffect(() => {
    if (open) {
      setName(query.slice(0, 40));
      setInlineError(null);
      saveSearch.reset();
    }
  }, [open, query]); // eslint-disable-line react-hooks/exhaustive-deps

  // Autofocus the name input.
  useEffect(() => {
    if (open) {
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  // Close on Escape.
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open) return null;

  async function handleSave() {
    const trimmedName = name.trim();
    if (!trimmedName) return;
    setInlineError(null);

    try {
      await saveSearch.mutateAsync({
        name: trimmedName,
        query,
        mode: searchMode,
        accountId: null,
      });
      onClose();
    } catch (err) {
      const e = err as { code?: string };
      if (e?.code === "DB_CONSTRAINT") {
        setInlineError(t("err_search_name_dup"));
      } else {
        setInlineError((err as Error)?.message ?? "Something went wrong.");
      }
    }
  }

  return (
    <div
      className="bg-p10/40 fixed inset-0 z-[60] flex items-center justify-center p-4"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="w-full max-w-sm rounded-card bg-surface p-5 shadow-card"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t("save_action")}
      >
        <h2 className="font-display text-lg italic text-p10">{t("save_action")}</h2>

        <div className="mt-4">
          <input
            ref={inputRef}
            type="text"
            value={name}
            onChange={(e) => {
              setName(e.target.value);
              setInlineError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") void handleSave();
            }}
            placeholder={t("save_name_placeholder")}
            className={cn(
              "w-full rounded-chip border bg-surface px-3 py-2 font-body text-sm text-p10",
              "placeholder:text-p7 focus:outline-none focus:ring-1",
              inlineError ? "border-red focus:ring-red" : "border-divider focus:ring-p7",
            )}
            maxLength={80}
          />
          {inlineError && <p className="mt-1.5 font-ui text-xs text-red">{inlineError}</p>}
        </div>

        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p4"
          >
            {t("save_cancel")}
          </button>
          <button
            type="button"
            onClick={() => void handleSave()}
            disabled={!name.trim() || saveSearch.isPending}
            className={cn(
              "rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white",
              "bg-p9 hover:bg-p10 disabled:cursor-not-allowed disabled:opacity-40",
            )}
          >
            {saveSearch.isPending ? "…" : t("save_confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
