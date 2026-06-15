// Saved-search list (T035). Self-contained — mounts wherever the integrator
// drops it (Sidebar, settings panel, etc.). Renders the useSavedSearches() list
// with per-row delete buttons and click-to-run support. No drag reorder in this
// iteration (requires @dnd-kit/sortable which is not yet in dependencies).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";

import type { SavedSearch } from "@shared/bindings";
import { useDeleteSavedSearch, useSavedSearches } from "@/ipc/queries/search";
import { useUi } from "@/stores/ui";
import { cn } from "@/lib/cn";

// ── Mode icon helpers ─────────────────────────────────────────────────────────

function ModeIcon({ mode }: { mode: string }) {
  if (mode === "keyword") {
    return (
      <span aria-hidden className="font-mono text-[10px] text-p7">
        KW
      </span>
    );
  }
  if (mode === "semantic") {
    return (
      <span aria-hidden className="font-mono text-[10px] text-p7">
        SEM
      </span>
    );
  }
  // auto or anything else
  return (
    <span aria-hidden className="font-mono text-[10px] text-p7">
      AUTO
    </span>
  );
}

// ── Delete confirm inline ─────────────────────────────────────────────────────

interface SavedSearchRowProps {
  item: SavedSearch;
  onRun: (item: SavedSearch) => void;
}

function SavedSearchRow({ item, onRun }: SavedSearchRowProps) {
  const { t } = useTranslation("search");
  const deleteMutation = useDeleteSavedSearch();
  const [confirmOpen, setConfirmOpen] = useState(false);

  function handleDelete(e: React.MouseEvent) {
    e.stopPropagation();
    if (!confirmOpen) {
      setConfirmOpen(true);
      return;
    }
    deleteMutation.mutate(item.id);
    setConfirmOpen(false);
  }

  function handleBlurCancel() {
    // Give a tick for the click to register before closing.
    setTimeout(() => setConfirmOpen(false), 150);
  }

  return (
    <div
      className={cn(
        "group flex items-center gap-2 rounded-chip px-2 py-1.5 transition-colors hover:bg-p4",
      )}
    >
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-2 text-start"
        onClick={() => onRun(item)}
        title={item.query}
      >
        <ModeIcon mode={item.mode} />
        <span className="min-w-0 truncate font-ui text-xs text-p9">{item.name}</span>
      </button>

      <div className="relative shrink-0">
        {confirmOpen && (
          <span className="me-1 font-ui text-[10px] text-red">
            {t("confirm_delete_saved_search")}
          </span>
        )}
        <button
          type="button"
          onClick={handleDelete}
          onBlur={handleBlurCancel}
          aria-label={`${t("btn_delete")} "${item.name}"`}
          className={cn(
            "rounded-chip p-0.5 opacity-0 transition-opacity group-hover:opacity-100",
            confirmOpen ? "text-red opacity-100" : "text-p7 hover:text-red",
          )}
        >
          <Trash2 size={12} aria-hidden />
        </button>
      </div>
    </div>
  );
}

// ── SavedSearchList ───────────────────────────────────────────────────────────

export interface SavedSearchListProps {
  /** Called when the user clicks a saved search to run it. */
  onRun?: (query: string, mode: string) => void;
}

export function SavedSearchList({ onRun }: SavedSearchListProps) {
  const { t } = useTranslation("search");
  const setSearchMode = useUi((s) => s.setSearchMode);
  const { data: items = [], isLoading } = useSavedSearches();

  function handleRun(item: SavedSearch) {
    // Switch the store mode, then hand the query up so the /search page runs it.
    const mode = item.mode === "keyword" ? "keyword" : "semantic";
    setSearchMode(mode);
    onRun?.(item.query, item.mode);
  }

  if (isLoading) {
    return (
      <div className="px-2 py-1">
        <div className="h-3 w-3/5 animate-pulse rounded bg-p5" />
      </div>
    );
  }

  if (items.length === 0) {
    return <p className="px-2 py-1 font-ui text-xs text-p7">{t("saved_title")} —</p>;
  }

  return (
    <div className="flex flex-col gap-0.5">
      {items.map((item) => (
        <SavedSearchRow key={item.id} item={item} onRun={handleRun} />
      ))}
    </div>
  );
}
