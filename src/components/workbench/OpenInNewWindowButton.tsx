// Persistent "open in new window" affordance (WB-20). A small ⤢ icon button that detaches the
// given workspace into its own OS window (T2). Drop it on spacious surfaces — the reading pane
// header, the compose top bar — where a persistent control fits. List rows use the right-click
// menu (WB-19) and double-click (WB-21) instead. Off-Tauri the click is a silent no-op.
import { SquareArrowOutUpRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/cn";
import type { TabSpec } from "@/stores/workbench";
import { useOpenInNewWindow } from "./useOpenInNewWindow";

export interface OpenInNewWindowButtonProps {
  spec: TabSpec;
  className?: string;
}

export default function OpenInNewWindowButton({ spec, className }: OpenInNewWindowButtonProps) {
  const { t } = useTranslation("common");
  const openInNewWindow = useOpenInNewWindow();
  const label = t("wb_open_in_new_window", "Open in new window");
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={(e) => {
        e.stopPropagation();
        openInNewWindow(spec);
      }}
      className={cn(
        "grid h-7 w-7 shrink-0 place-items-center rounded-[6px] text-p7 transition-colors hover:bg-surface hover:text-p10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate",
        className,
      )}
    >
      <SquareArrowOutUpRight className="h-4 w-4" />
    </button>
  );
}
