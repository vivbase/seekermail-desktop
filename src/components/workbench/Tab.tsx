// One workbench tab chip (WB-03 / WB-07). Account-color dot + ellipsized title + dirty
// indicator; a pin icon when pinned. Hover/focus reveals ⤢ (open in new window — detach,
// wired by WB-18/WB-20) and × (close). Right-click opens the pin/close context menu (handled
// by TabStrip). The chip is `role="tab"` (not a <button>) so it can nest the action buttons.
import { type DragEvent, type KeyboardEvent, type MouseEvent } from "react";
import { X, SquareArrowOutUpRight, Pin } from "lucide-react";

import { accountColorClass, type AccountColorToken } from "@/lib/accountColor";
import { cn } from "@/lib/cn";

export interface TabProps {
  title: string;
  active: boolean;
  colorToken?: AccountColorToken;
  dirty?: boolean;
  pinned?: boolean;
  closeLabel: string;
  detachLabel: string;
  onActivate: () => void;
  onClose: () => void;
  /** ⤢ open in new window. Omitted → the detach affordance is hidden. */
  onDetach?: () => void;
  onContextMenu?: (e: MouseEvent<HTMLDivElement>) => void;
  draggable?: boolean;
  onDragStart?: (e: DragEvent<HTMLDivElement>) => void;
  onDragOver?: (e: DragEvent<HTMLDivElement>) => void;
  onDrop?: (e: DragEvent<HTMLDivElement>) => void;
  /** Register the chip element (focus management after close, WB-04). */
  tabRef?: (el: HTMLDivElement | null) => void;
}

const ACTION_BTN =
  "grid h-4 w-4 shrink-0 place-items-center rounded-[4px] text-p7 opacity-0 transition-opacity hover:bg-p3 hover:text-p10 focus:opacity-100 focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100";

export default function Tab({
  title,
  active,
  colorToken,
  dirty,
  pinned,
  closeLabel,
  detachLabel,
  onActivate,
  onClose,
  onDetach,
  onContextMenu,
  draggable,
  onDragStart,
  onDragOver,
  onDrop,
  tabRef,
}: TabProps) {
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onActivate();
    }
  };

  return (
    <div
      ref={tabRef}
      role="tab"
      aria-selected={active}
      aria-label={title}
      tabIndex={0}
      onClick={onActivate}
      onKeyDown={onKeyDown}
      onContextMenu={onContextMenu}
      draggable={draggable}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDrop={onDrop}
      className={cn(
        "group flex h-8 min-w-[112px] max-w-[200px] shrink-0 cursor-pointer items-center gap-2 rounded-t-[7px] border border-b-0 pl-2.5 pr-2 font-ui text-[11.5px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-slate",
        active
          ? "border-divider bg-surface text-p10"
          : "hover:bg-surface/60 border-transparent text-p8",
      )}
    >
      {pinned ? <Pin aria-hidden data-pin-icon className="h-3 w-3 shrink-0 text-p8" /> : null}
      <span
        aria-hidden
        className={cn(
          "h-2 w-2 shrink-0 rounded-avatar",
          colorToken ? accountColorClass(colorToken) : "bg-p7",
        )}
      />
      <span className="flex-1 truncate">{title}</span>
      <span className="flex shrink-0 items-center gap-0.5">
        {dirty ? (
          <span
            aria-hidden
            data-dirty-dot
            className="mr-0.5 h-1.5 w-1.5 rounded-avatar bg-p7 group-focus-within:hidden group-hover:hidden"
          />
        ) : null}
        {onDetach ? (
          <button
            type="button"
            aria-label={detachLabel}
            className={ACTION_BTN}
            onClick={(e) => {
              e.stopPropagation();
              onDetach();
            }}
          >
            <SquareArrowOutUpRight className="h-3 w-3" />
          </button>
        ) : null}
        <button
          type="button"
          aria-label={closeLabel}
          className={cn(ACTION_BTN, "hover:bg-red hover:text-white")}
          onClick={(e) => {
            e.stopPropagation();
            onClose();
          }}
        >
          <X className="h-3 w-3" />
        </button>
      </span>
    </div>
  );
}
