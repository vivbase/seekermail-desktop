// Tokenized LCS word diff (T090, F_E6 §4.5). Pure + dependency-free: the
// sandboxed build cannot reliably add `diff-match-patch`, and a word-level
// longest-common-subsequence is all the draft diff view needs. Inputs are
// PLAIN TEXT (AiDraft.bodyOriginal / bodyCurrent) — never HTML.

export type DiffOp = "equal" | "insert" | "delete";

export interface DiffSegment {
  op: DiffOp;
  text: string;
}

/**
 * Above this token-matrix size the O(n·m) LCS table is skipped and the diff
 * degrades to one delete + one insert block. AI drafts are a few hundred
 * words, so this guard only protects against pathological inputs.
 */
const MAX_LCS_CELLS = 1_000_000;

/** Split into word + whitespace tokens so the LCS aligns on word boundaries. */
export function tokenize(text: string): string[] {
  if (text === "") return [];
  return text.split(/(\s+)/).filter((t) => t !== "");
}

/**
 * Word-level diff of two plain-text strings. Adjacent segments of the same op
 * are merged, so the result is the minimal run-length segment list. Equal
 * inputs return a single `equal` segment (or `[]` for two empty strings).
 */
export function diffText(original: string, current: string): DiffSegment[] {
  if (original === current) {
    return original === "" ? [] : [{ op: "equal", text: original }];
  }

  const a = tokenize(original);
  const b = tokenize(current);

  if (a.length * b.length > MAX_LCS_CELLS) {
    return mergeSegments([
      { op: "delete", text: original },
      { op: "insert", text: current },
    ]);
  }

  // LCS length table: lcs[i][j] = LCS length of a[i..] vs b[j..].
  const cols = b.length + 1;
  const lcs = new Uint32Array((a.length + 1) * cols);
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      lcs[i * cols + j] =
        a[i] === b[j]
          ? lcs[(i + 1) * cols + j + 1]! + 1
          : Math.max(lcs[(i + 1) * cols + j]!, lcs[i * cols + j + 1]!);
    }
  }

  // Walk the table to emit ops in order.
  const segments: DiffSegment[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      segments.push({ op: "equal", text: a[i]! });
      i++;
      j++;
    } else if (lcs[(i + 1) * cols + j]! >= lcs[i * cols + j + 1]!) {
      segments.push({ op: "delete", text: a[i]! });
      i++;
    } else {
      segments.push({ op: "insert", text: b[j]! });
      j++;
    }
  }
  while (i < a.length) {
    segments.push({ op: "delete", text: a[i]! });
    i++;
  }
  while (j < b.length) {
    segments.push({ op: "insert", text: b[j]! });
    j++;
  }

  return mergeSegments(segments);
}

/** True when the diff contains any non-equal segment. */
export function hasChanges(segments: DiffSegment[]): boolean {
  return segments.some((s) => s.op !== "equal");
}

function mergeSegments(segments: DiffSegment[]): DiffSegment[] {
  const merged: DiffSegment[] = [];
  for (const seg of segments) {
    const last = merged[merged.length - 1];
    if (last && last.op === seg.op) last.text += seg.text;
    else merged.push({ ...seg });
  }
  return merged;
}
