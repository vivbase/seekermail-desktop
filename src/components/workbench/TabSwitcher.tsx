// Tab quick-switcher palette (WB-08). A lightweight command-palette over the open tabs:
// filter by title, arrow-key to move, Enter to jump (activateTab), Esc/backdrop to close.
// Opened from the Cmd/Ctrl+P shortcut (Cmd+K is taken by Search). Controlled by the parent.
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

import { useWorkbench } from "@/stores/workbench";
import { cn } from "@/lib/cn";

export interface TabSwitcherProps {
  open: boolean;
  onClose: () => void;
}

export default function TabSwitcher({ open, onClose }: TabSwitcherProps) {
  const { t } = useTranslation("common");
  const tabs = useWorkbench((s) => s.tabs);
  const activateTab = useWorkbench((s) => s.activateTab);
  const [q, setQ] = useState("");
  const [idx, setIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const filtered = tabs.filter((tb) => tb.title.toLowerCase().includes(q.trim().toLowerCase()));

  useEffect(() => {
    if (open) {
      setQ("");
      setIdx(0);
      inputRef.current?.focus();
    }
  }, [open]);

  if (!open) return null;

  const choose = (id?: string) => {
    if (id) activateTab(id);
    onClose();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setIdx((i) => Math.min(i + 1, Math.max(filtered.length - 1, 0)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      choose(filtered[idx]?.id);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/20 pt-[12vh]"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-label={t("wb_switch_tab", "Switch tab")}
        className="w-[460px] max-w-[92vw] overflow-hidden rounded-card border border-divider bg-surface shadow-card"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <input
          ref={inputRef}
          value={q}
          onChange={(e) => {
            setQ(e.target.value);
            setIdx(0);
          }}
          placeholder={t("wb_switch_placeholder", "Switch to tab…")}
          aria-label={t("wb_switch_tab", "Switch tab")}
          className="w-full border-b border-divider bg-transparent px-4 py-3 font-ui text-sm text-p10 outline-none"
        />
        <ul role="listbox" className="max-h-[50vh] overflow-y-auto py-1">
          {filtered.length === 0 ? (
            <li className="px-4 py-3 font-ui text-xs text-p7">
              {t("wb_switch_empty", "No matching tabs")}
            </li>
          ) : (
            filtered.map((tb, i) => (
              <li
                key={tb.id}
                role="option"
                aria-selected={i === idx}
                data-testid={`switch-${tb.id}`}
                onMouseEnter={() => setIdx(i)}
                onClick={() => choose(tb.id)}
                className={cn(
                  "cursor-pointer px-4 py-2 font-ui text-[13px]",
                  i === idx ? "bg-parchment text-p10" : "text-p8",
                )}
              >
                {tb.title}
              </li>
            ))
          )}
        </ul>
      </div>
    </div>
  );
}
