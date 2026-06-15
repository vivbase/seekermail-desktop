// Virtualized search result list (T034, extended for attachment hits in T110).
// Uses TanStack Virtual for the mail hits; attachment hits render in a labeled
// section below (small N, not virtualized). Renders loading skeleton, empty
// state, result count, and delegates per-row rendering to SearchResultCard.
import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";

import type { AttachmentHit, PageResult, SearchResult } from "@shared/bindings";
import { SearchResultCard } from "./SearchResultCard";

// ── Loading skeleton ──────────────────────────────────────────────────────────

function SkeletonRow() {
  return (
    <div className="flex flex-col gap-2 rounded-chip px-4 py-3">
      <div className="flex items-center justify-between gap-3">
        <div className="h-3 w-3/5 animate-pulse rounded bg-p5" />
        <div className="h-3.5 w-10 animate-pulse rounded-chip bg-p5" />
      </div>
      <div className="flex items-center justify-between gap-3">
        <div className="h-2.5 w-1/4 animate-pulse rounded bg-p5" />
        <div className="h-2.5 w-12 animate-pulse rounded bg-p5" />
      </div>
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div className="flex flex-col">
      {[0, 1, 2].map((i) => (
        <SkeletonRow key={i} />
      ))}
    </div>
  );
}

// ── SearchResultList ──────────────────────────────────────────────────────────

export interface SearchResultListProps {
  data: PageResult<SearchResult> | undefined;
  isLoading: boolean;
  isEmpty: boolean;
  /** If true, show the semantic-fallback hint above keyword results. */
  showSemanticFallback?: boolean;
  selectedIndex: number;
  onSelect: (result: SearchResult, index: number) => void;
  /** Attachment-origin hits (T110); rendered in a labeled section below mail hits. */
  attachmentHits?: AttachmentHit[];
  /** Click handler for an attachment hit (navigates to the L2 mail + highlight). */
  onSelectAttachment?: (hit: AttachmentHit) => void;
  /** Optional max-height for the virtualized scroll area (page vs overlay layout). */
  listMaxHeight?: string;
}

export function SearchResultList({
  data,
  isLoading,
  isEmpty,
  showSemanticFallback = false,
  selectedIndex,
  onSelect,
  attachmentHits,
  onSelectAttachment,
  listMaxHeight,
}: SearchResultListProps) {
  const { t } = useTranslation("search");
  const parentRef = useRef<HTMLDivElement>(null);
  const items = data?.items ?? [];
  const attachmentItems = attachmentHits ?? [];

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 72,
    overscan: 5,
    measureElement: (el) => el.getBoundingClientRect().height,
  });

  if (isLoading) {
    return <LoadingSkeleton />;
  }

  // Empty only when neither mail hits nor attachment hits exist.
  if ((isEmpty || items.length === 0) && attachmentItems.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 py-10 text-center">
        <p className="font-ui text-sm font-semibold text-p8">{t("empty_title")}</p>
        <p className="max-w-xs font-body text-xs text-p7">{t("empty_hint")}</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col">
      {/* Result count + semantic fallback hint */}
      {items.length > 0 && (
        <div className="flex items-center justify-between gap-2 px-4 pb-2 pt-1">
          <p className="font-ui text-xs text-p7">
            {t("result_count", { count: data?.total ?? items.length })}
          </p>
          {showSemanticFallback && (
            <span className="font-ui text-xs text-amber">{t("semantic_fallback")}</span>
          )}
        </div>
      )}

      {/* Virtualized mail-hit list */}
      {items.length > 0 && (
        <div
          ref={parentRef}
          className="overflow-y-auto"
          style={{ maxHeight: listMaxHeight ?? "min(calc(100vh - 320px), 380px)" }}
        >
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              position: "relative",
            }}
          >
            {virtualizer.getVirtualItems().map((vItem) => {
              const result = items[vItem.index];
              if (!result) return null;
              return (
                <div
                  key={vItem.key}
                  data-index={vItem.index}
                  ref={virtualizer.measureElement}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    right: 0,
                    transform: `translateY(${vItem.start}px)`,
                  }}
                >
                  <SearchResultCard
                    result={result}
                    selected={selectedIndex === vItem.index}
                    onClick={() => onSelect(result, vItem.index)}
                  />
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Attachment-hit section (T110) — visually distinct, labeled. */}
      {attachmentItems.length > 0 && (
        <div className="border-t border-divider pt-1">
          <p className="section-label px-4 pb-1 pt-1">{t("search_attachment_hits_label")}</p>
          <div className="max-h-60 overflow-y-auto">
            {attachmentItems.map((hit) => (
              <SearchResultCard
                key={hit.attachmentId}
                source="attachment"
                hit={hit}
                selected={false}
                onClick={() => onSelectAttachment?.(hit)}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
