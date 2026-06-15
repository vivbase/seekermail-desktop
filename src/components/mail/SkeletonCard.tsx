// Loading skeleton placeholder (T037). Matches the 72 px ThreadCard row height
// and uses design-token colours only — no bare hex.
import { cn } from "@/lib/cn";

interface SkeletonCardProps {
  /** Compact variant matches density="compact" (56 px). Defaults to comfortable (72 px). */
  compact?: boolean;
}

export function SkeletonCard({ compact = false }: SkeletonCardProps) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "flex w-full items-center gap-3 border-b border-divider bg-surface px-4",
        compact ? "h-14" : "h-[72px]",
      )}
    >
      {/* Account color stripe */}
      <div className="h-full w-[3px] shrink-0 rounded-full bg-p5" />

      {/* Avatar circle */}
      <div className="h-9 w-9 shrink-0 animate-pulse rounded-avatar bg-p5" />

      {/* Text lines */}
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div className="flex items-center justify-between gap-2">
          <div className="h-3 w-28 animate-pulse rounded bg-p5" />
          <div className="h-3 w-12 animate-pulse rounded bg-p5" />
        </div>
        <div className="h-3 w-48 animate-pulse rounded bg-p5" />
        <div className="h-2.5 w-full max-w-xs animate-pulse rounded bg-p4" />
      </div>
    </div>
  );
}
