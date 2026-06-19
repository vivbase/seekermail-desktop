// Rich-text formatting toolbar for the compose editor (T044, F_G4 §4.4). Drives
// the sibling contentEditable surface through the editing commands the platform
// webview exposes (bold/italic/underline, font size, text + highlight colour,
// lists, indent, alignment, quote, link, clear formatting, undo/redo) — the
// Gmail-equivalent baseline. Every control acts on `mousedown` with
// preventDefault so the editor never loses its selection, and the live caret
// range is mirrored into a ref so focus-stealing popovers can restore it.

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/cn";

// ── Types ────────────────────────────────────────────────────────────────────

interface ComposeFormatBarProps {
  /** The contentEditable surface this toolbar formats. */
  editorRef: React.RefObject<HTMLDivElement | null>;
}

type MenuId = "size" | "color" | "highlight" | "link";

// ── Palettes & sizes ─────────────────────────────────────────────────────────

/** Text colours offered in the foreground-colour popover. */
const TEXT_COLORS: { label: string; value: string }[] = [
  { label: "Default", value: "#231F1A" },
  { label: "Grey", value: "#7A7268" },
  { label: "Red", value: "#C0392B" },
  { label: "Terracotta", value: "#C05A38" },
  { label: "Amber", value: "#B7791F" },
  { label: "Green", value: "#4A7C59" },
  { label: "Teal", value: "#2C7A7B" },
  { label: "Blue", value: "#2C5D8F" },
  { label: "Indigo", value: "#434190" },
  { label: "Purple", value: "#6B4A9C" },
];

/** Highlight (background) colours offered in the highlight popover. */
const HIGHLIGHT_COLORS: { label: string; value: string }[] = [
  { label: "None", value: "transparent" },
  { label: "Yellow", value: "#FFF3BF" },
  { label: "Green", value: "#D3F9D8" },
  { label: "Blue", value: "#D0EBFF" },
  { label: "Pink", value: "#FFD8E4" },
  { label: "Orange", value: "#FFE8CC" },
  { label: "Purple", value: "#E5DBFF" },
];

/** Font sizes mapped to the legacy `fontSize` 1–7 scale the command expects. */
const FONT_SIZES: { label: string; value: string }[] = [
  { label: "Small", value: "2" },
  { label: "Normal", value: "3" },
  { label: "Large", value: "5" },
  { label: "Huge", value: "7" },
];

// ── Icon helpers ─────────────────────────────────────────────────────────────

const ICON = "h-3.5 w-3.5";

function Svg({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={cn(ICON, className)}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/** A single command button. Acts on mousedown so the editor keeps its selection. */
function TBButton({
  label,
  onMouseDown,
  isActive,
  children,
}: {
  label: string;
  onMouseDown: (e: React.MouseEvent) => void;
  isActive?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={isActive ?? undefined}
      onMouseDown={onMouseDown}
      className={cn(
        "flex h-7 min-w-7 items-center justify-center rounded-chip px-1.5 text-p8 transition-colors",
        "hover:bg-p4 hover:text-p10 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9",
        isActive && "bg-p4 text-p10",
      )}
    >
      {children}
    </button>
  );
}

// ── Component ────────────────────────────────────────────────────────────────

export function ComposeFormatBar({ editorRef }: ComposeFormatBarProps) {
  const { t } = useTranslation("compose");
  const [active, setActive] = useState<Record<string, boolean>>({});
  const [sizeValue, setSizeValue] = useState("3");
  const [openMenu, setOpenMenu] = useState<MenuId | null>(null);
  const [linkUrl, setLinkUrl] = useState("");

  const savedRangeRef = useRef<Range | null>(null);
  const barRef = useRef<HTMLDivElement>(null);

  // ── Selection plumbing ───────────────────────────────────────────────────

  const selectionInEditor = useCallback((): boolean => {
    const el = editorRef.current;
    const sel = window.getSelection();
    if (!el || !sel || sel.rangeCount === 0) return false;
    return el.contains(sel.anchorNode);
  }, [editorRef]);

  const saveSelection = useCallback(() => {
    const sel = window.getSelection();
    if (sel && sel.rangeCount > 0 && selectionInEditor()) {
      savedRangeRef.current = sel.getRangeAt(0).cloneRange();
    }
  }, [selectionInEditor]);

  const restoreSelection = useCallback(() => {
    const el = editorRef.current;
    if (!el) return;
    el.focus();
    const sel = window.getSelection();
    if (savedRangeRef.current && sel) {
      sel.removeAllRanges();
      sel.addRange(savedRangeRef.current);
    }
  }, [editorRef]);

  /** Read the current command states so buttons can show their active styling. */
  const refreshActive = useCallback(() => {
    if (!selectionInEditor()) return;
    const read = (cmd: string): boolean => {
      try {
        return document.queryCommandState(cmd);
      } catch {
        return false;
      }
    };
    setActive({
      bold: read("bold"),
      italic: read("italic"),
      underline: read("underline"),
      strikeThrough: read("strikeThrough"),
      insertUnorderedList: read("insertUnorderedList"),
      insertOrderedList: read("insertOrderedList"),
      justifyLeft: read("justifyLeft"),
      justifyCenter: read("justifyCenter"),
      justifyRight: read("justifyRight"),
    });
    try {
      const sz = document.queryCommandValue("fontSize");
      if (sz) setSizeValue(String(sz));
    } catch {
      /* queryCommandValue unsupported — leave the last known size. */
    }
  }, [selectionInEditor]);

  // Mirror the in-editor caret and refresh active states on every change.
  useEffect(() => {
    function onSelChange() {
      if (selectionInEditor()) {
        saveSelection();
        refreshActive();
      }
    }
    document.addEventListener("selectionchange", onSelChange);
    return () => document.removeEventListener("selectionchange", onSelChange);
  }, [selectionInEditor, saveSelection, refreshActive]);

  // Close any open popover on outside-click or Escape.
  useEffect(() => {
    if (!openMenu) return;
    function onDown(e: MouseEvent) {
      if (barRef.current && !barRef.current.contains(e.target as Node)) setOpenMenu(null);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpenMenu(null);
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [openMenu]);

  // ── Command execution ────────────────────────────────────────────────────

  const exec = useCallback(
    (command: string, value?: string) => {
      restoreSelection();
      try {
        // Emit inline-styled markup (CSS) rather than legacy <font> tags so the
        // outgoing text/html part stays portable across mail clients.
        document.execCommand("styleWithCSS", false, "true");
      } catch {
        /* styleWithCSS unsupported on this engine — fall through. */
      }
      try {
        document.execCommand(command, false, value);
      } catch {
        /* Command unsupported on this engine — no-op rather than crash. */
      }
      saveSelection();
      refreshActive();
      // Notify the editor so it re-reads innerHTML/innerText into the store.
      editorRef.current?.dispatchEvent(new Event("input", { bubbles: true }));
    },
    [restoreSelection, saveSelection, refreshActive, editorRef],
  );

  /** Build the mousedown handler shared by every command button. */
  function cmd(command: string, value?: string) {
    return (e: React.MouseEvent) => {
      e.preventDefault();
      exec(command, value);
    };
  }

  function applyColor(command: "foreColor" | "hiliteColor", value: string) {
    // With styleWithCSS enabled (see exec), both WebKit and Chromium apply
    // hiliteColor as an inline background on the selection.
    exec(command, value);
    setOpenMenu(null);
  }

  function applyLink() {
    const url = linkUrl.trim();
    if (!url) return;
    const href = /^(https?:|mailto:)/i.test(url) ? url : `https://${url}`;
    restoreSelection();
    const sel = window.getSelection();
    const collapsed = !sel || sel.isCollapsed;
    if (collapsed) {
      // No selection: drop an anchor whose visible text is the URL itself.
      const safe = href.replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
      exec("insertHTML", `<a href="${safe}">${safe}</a>`);
    } else {
      exec("createLink", href);
    }
    setLinkUrl("");
    setOpenMenu(null);
  }

  function toggleMenu(id: MenuId) {
    saveSelection();
    setOpenMenu((cur) => (cur === id ? null : id));
  }

  // ── Render ───────────────────────────────────────────────────────────────

  const sizeLabel = FONT_SIZES.find((s) => s.value === sizeValue)?.label ?? "Normal";
  const sepCls = "mx-0.5 h-5 w-px shrink-0 bg-divider";

  return (
    <div
      ref={barRef}
      role="toolbar"
      aria-label="Text formatting"
      className="sticky top-0 z-10 flex flex-wrap items-center gap-0.5 border-b border-divider bg-surface px-4 py-1.5"
    >
      {/* Undo / redo */}
      <TBButton label="Undo" onMouseDown={cmd("undo")}>
        <Svg>
          <path d="M3.5 8.5h7a3 3 0 0 1 0 6H7" />
          <path d="M6 5.5 3 8.5l3 3" />
        </Svg>
      </TBButton>
      <TBButton label="Redo" onMouseDown={cmd("redo")}>
        <Svg>
          <path d="M12.5 8.5h-7a3 3 0 0 0 0 6H9" />
          <path d="M10 5.5l3 3-3 3" />
        </Svg>
      </TBButton>

      <span className={sepCls} aria-hidden="true" />

      {/* Font size */}
      <div className="relative">
        <button
          type="button"
          title="Font size"
          aria-label="Font size"
          aria-haspopup="menu"
          aria-expanded={openMenu === "size"}
          onMouseDown={(e) => {
            e.preventDefault();
            toggleMenu("size");
          }}
          className="flex h-7 items-center gap-1 rounded-chip px-2 font-ui text-[10px] uppercase tracking-wider text-p8 transition-colors hover:bg-p4 hover:text-p10"
        >
          {sizeLabel}
          <Svg className="h-3 w-3">
            <path d="M4 6l4 4 4-4" />
          </Svg>
        </button>
        {openMenu === "size" && (
          <div
            role="menu"
            className="absolute left-0 top-8 z-20 w-32 rounded-chip border border-divider bg-surface py-1 shadow-card"
          >
            {FONT_SIZES.map((s) => (
              <button
                key={s.value}
                type="button"
                role="menuitemradio"
                aria-checked={sizeValue === s.value}
                onMouseDown={(e) => {
                  e.preventDefault();
                  exec("fontSize", s.value);
                  setOpenMenu(null);
                }}
                className={cn(
                  "flex w-full items-center px-3 py-1.5 text-left font-body text-p9 hover:bg-p4",
                  sizeValue === s.value && "bg-p4",
                )}
                style={{
                  fontSize: s.value === "2" ? 12 : s.value === "5" ? 17 : s.value === "7" ? 20 : 14,
                }}
              >
                {s.label}
              </button>
            ))}
          </div>
        )}
      </div>

      <span className={sepCls} aria-hidden="true" />

      {/* Bold / italic / underline / strikethrough */}
      <TBButton label="Bold" isActive={active.bold} onMouseDown={cmd("bold")}>
        <span className="font-ui text-sm font-bold">B</span>
      </TBButton>
      <TBButton label="Italic" isActive={active.italic} onMouseDown={cmd("italic")}>
        <span className="font-ui text-sm italic">I</span>
      </TBButton>
      <TBButton label="Underline" isActive={active.underline} onMouseDown={cmd("underline")}>
        <span className="font-ui text-sm underline">U</span>
      </TBButton>
      <TBButton
        label="Strikethrough"
        isActive={active.strikeThrough}
        onMouseDown={cmd("strikeThrough")}
      >
        <span className="font-ui text-sm line-through">S</span>
      </TBButton>

      {/* Text colour */}
      <div className="relative">
        <button
          type="button"
          title="Text color"
          aria-label="Text color"
          aria-haspopup="menu"
          aria-expanded={openMenu === "color"}
          onMouseDown={(e) => {
            e.preventDefault();
            toggleMenu("color");
          }}
          className="flex h-7 min-w-7 flex-col items-center justify-center rounded-chip px-1.5 text-p8 transition-colors hover:bg-p4 hover:text-p10"
        >
          <span className="font-ui text-sm leading-none">A</span>
          <span className="mt-0.5 h-1 w-3.5 rounded-sm" style={{ backgroundColor: "#C05A38" }} />
        </button>
        {openMenu === "color" && (
          <ColorPopover
            colors={TEXT_COLORS}
            onPick={(c) => applyColor("foreColor", c)}
            onCustom={(c) => applyColor("foreColor", c)}
          />
        )}
      </div>

      {/* Highlight colour */}
      <div className="relative">
        <button
          type="button"
          title="Highlight color"
          aria-label="Highlight color"
          aria-haspopup="menu"
          aria-expanded={openMenu === "highlight"}
          onMouseDown={(e) => {
            e.preventDefault();
            toggleMenu("highlight");
          }}
          className="flex h-7 min-w-7 items-center justify-center rounded-chip px-1.5 text-p8 transition-colors hover:bg-p4 hover:text-p10"
        >
          <Svg>
            <path d="M9.5 3.5l3 3-5.5 5.5H4v-3z" />
            <path d="M2.5 14.5h11" />
          </Svg>
        </button>
        {openMenu === "highlight" && (
          <ColorPopover
            colors={HIGHLIGHT_COLORS}
            onPick={(c) => applyColor("hiliteColor", c)}
            onCustom={(c) => applyColor("hiliteColor", c)}
          />
        )}
      </div>

      <span className={sepCls} aria-hidden="true" />

      {/* Lists */}
      <TBButton
        label="Bulleted list"
        isActive={active.insertUnorderedList}
        onMouseDown={cmd("insertUnorderedList")}
      >
        <Svg>
          <path d="M6 4.5h7M6 8h7M6 11.5h7" />
          <circle cx="3" cy="4.5" r="0.6" fill="currentColor" stroke="none" />
          <circle cx="3" cy="8" r="0.6" fill="currentColor" stroke="none" />
          <circle cx="3" cy="11.5" r="0.6" fill="currentColor" stroke="none" />
        </Svg>
      </TBButton>
      <TBButton
        label="Numbered list"
        isActive={active.insertOrderedList}
        onMouseDown={cmd("insertOrderedList")}
      >
        <Svg>
          <path d="M6.5 4.5h7M6.5 8h7M6.5 11.5h7" />
          <path
            d="M2.4 3.2h.8v2.4M2.2 8.2h1.2L2.2 9.7h1.2M2.3 11h1v.9h-1m0 .9h1v.9h-1"
            strokeWidth="1"
          />
        </Svg>
      </TBButton>

      {/* Indent */}
      <TBButton label="Decrease indent" onMouseDown={cmd("outdent")}>
        <Svg>
          <path d="M13 4.5H7M13 8H9M13 11.5H7" />
          <path d="M5 6L2.5 8 5 10" />
        </Svg>
      </TBButton>
      <TBButton label="Increase indent" onMouseDown={cmd("indent")}>
        <Svg>
          <path d="M3 4.5h6M3 8h4M3 11.5h6" />
          <path d="M11 6l2.5 2L11 10" />
        </Svg>
      </TBButton>

      <span className={sepCls} aria-hidden="true" />

      {/* Alignment */}
      <TBButton label="Align left" isActive={active.justifyLeft} onMouseDown={cmd("justifyLeft")}>
        <Svg>
          <path d="M2.5 4h11M2.5 8h7M2.5 12h9" />
        </Svg>
      </TBButton>
      <TBButton
        label="Align center"
        isActive={active.justifyCenter}
        onMouseDown={cmd("justifyCenter")}
      >
        <Svg>
          <path d="M2.5 4h11M4.5 8h7M3.5 12h9" />
        </Svg>
      </TBButton>
      <TBButton
        label="Align right"
        isActive={active.justifyRight}
        onMouseDown={cmd("justifyRight")}
      >
        <Svg>
          <path d="M2.5 4h11M6.5 8h7M4.5 12h9" />
        </Svg>
      </TBButton>

      {/* Quote */}
      <TBButton label="Quote" onMouseDown={cmd("formatBlock", "blockquote")}>
        <Svg>
          <path d="M6 5c-1.5 0-2.5 1-2.5 2.5S4.5 10 5.5 10c0 1-.5 1.5-1.5 2M12.5 5c-1.5 0-2.5 1-2.5 2.5S11 10 12 10c0 1-.5 1.5-1.5 2" />
        </Svg>
      </TBButton>

      {/* Link */}
      <div className="relative">
        <button
          type="button"
          title="Insert link"
          aria-label="Insert link"
          aria-haspopup="dialog"
          aria-expanded={openMenu === "link"}
          onMouseDown={(e) => {
            e.preventDefault();
            toggleMenu("link");
          }}
          className="flex h-7 min-w-7 items-center justify-center rounded-chip px-1.5 text-p8 transition-colors hover:bg-p4 hover:text-p10"
        >
          <Svg>
            <path d="M6.5 9.5l3-3M7 5l1-1a2.5 2.5 0 0 1 3.5 3.5l-1 1M9 11l-1 1A2.5 2.5 0 0 1 4.5 8.5l1-1" />
          </Svg>
        </button>
        {openMenu === "link" && (
          <div
            role="dialog"
            aria-label="Insert link"
            className="absolute left-0 top-8 z-20 w-64 rounded-chip border border-divider bg-surface p-2.5 shadow-card"
          >
            <input
              type="url"
              value={linkUrl}
              autoFocus
              placeholder={t("link_placeholder")}
              onChange={(e) => setLinkUrl(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  applyLink();
                }
              }}
              className="w-full rounded-chip border border-divider bg-parchment px-2 py-1 font-body text-sm text-p10 placeholder:text-p7 focus:outline-none focus:ring-1 focus:ring-p9"
            />
            <div className="mt-2 flex justify-end gap-2">
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  exec("unlink");
                  setOpenMenu(null);
                }}
                className="rounded-chip px-2 py-1 font-ui text-[10px] uppercase tracking-wider text-p7 hover:bg-p4 hover:text-p10"
              >
                {t("link_remove")}
              </button>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  applyLink();
                }}
                className="rounded-chip bg-p9 px-2.5 py-1 font-ui text-[10px] uppercase tracking-wider text-white hover:bg-p10"
              >
                {t("link_apply")}
              </button>
            </div>
          </div>
        )}
      </div>

      <span className={sepCls} aria-hidden="true" />

      {/* Clear formatting */}
      <TBButton label="Clear formatting" onMouseDown={cmd("removeFormat")}>
        <Svg>
          <path d="M5 4h8M8.5 4l-2 9M4 14h5" />
          <path d="M11.5 10.5l3 3M14.5 10.5l-3 3" strokeWidth="1.2" />
        </Svg>
      </TBButton>
    </div>
  );
}

// ── Colour popover ───────────────────────────────────────────────────────────

function ColorPopover({
  colors,
  onPick,
  onCustom,
}: {
  colors: { label: string; value: string }[];
  onPick: (value: string) => void;
  onCustom: (value: string) => void;
}) {
  const { t } = useTranslation("compose");
  return (
    <div
      role="menu"
      className="absolute left-0 top-8 z-20 w-44 rounded-chip border border-divider bg-surface p-2 shadow-card"
    >
      <div className="grid grid-cols-5 gap-1.5">
        {colors.map((c) => (
          <button
            key={c.label}
            type="button"
            role="menuitem"
            title={c.label}
            aria-label={c.label}
            onMouseDown={(e) => {
              e.preventDefault();
              onPick(c.value);
            }}
            className="h-6 w-6 rounded-chip border border-divider transition-transform hover:scale-110 focus:outline-none focus-visible:ring-2 focus-visible:ring-p9"
            style={
              c.value === "transparent"
                ? {
                    backgroundImage:
                      "linear-gradient(45deg, var(--p5) 25%, transparent 25%, transparent 75%, var(--p5) 75%)",
                    backgroundSize: "8px 8px",
                  }
                : { backgroundColor: c.value }
            }
          />
        ))}
      </div>
      <label className="mt-2 flex cursor-pointer items-center gap-2 border-t border-divider pt-2 font-ui text-[10px] uppercase tracking-wider text-p7">
        <input
          type="color"
          onChange={(e) => onCustom(e.target.value)}
          className="h-5 w-5 cursor-pointer rounded border border-divider bg-transparent p-0"
          aria-label="Custom color"
        />
        {t("color_custom")}
      </label>
    </div>
  );
}
