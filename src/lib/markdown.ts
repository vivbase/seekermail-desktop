// Minimal Markdown handling for AI-generated draft bodies (T078, F_E1 §4.3).
// Module E models output plain text with light Markdown (bold, paragraphs) and
// occasionally leave code-fence residue. Pure functions, no dependency — the
// compose editor is plain text today (T044), so seeds go through
// `markdownToPlainText`; `markdownToHtml` is the converter the Tiptap upgrade
// consumes when rich text lands (T044 v0.5+ note in ComposeEditor).

/** Strip ``` code-fence lines the model may leave around its output (F_E1 §4.3). */
export function stripCodeFences(markdown: string): string {
  return markdown
    .split("\n")
    .filter((line) => !/^\s*```/.test(line.trim()))
    .join("\n")
    .trim();
}

/** Escape the characters HTML treats specially before we add our own tags. */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/**
 * Convert light Markdown to editor-ready HTML: `**bold**` → `<strong>`,
 * blank-line-separated blocks → `<p>`, single newlines → `<br>`. Input is
 * HTML-escaped first, so model output can never inject markup.
 */
export function markdownToHtml(markdown: string): string {
  const cleaned = stripCodeFences(markdown);
  if (!cleaned) return "";
  return cleaned
    .split(/\n{2,}/)
    .map((block) => {
      const inline = escapeHtml(block)
        .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
        .replace(/\n/g, "<br>");
      return `<p>${inline}</p>`;
    })
    .join("");
}

/**
 * Reduce light Markdown to clean plain text for the v0.x textarea editor:
 * drops code fences and bold markers, keeps paragraph/line structure.
 */
export function markdownToPlainText(markdown: string): string {
  return stripCodeFences(markdown).replace(/\*\*([^*]+)\*\*/g, "$1");
}
