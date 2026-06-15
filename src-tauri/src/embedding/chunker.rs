//! Text chunking for the B3 embedding pipeline (T031, F_B3 §4.2).
//!
//! A pure, I/O-free function so it is exhaustively unit-testable. Strategy, in
//! descending preference:
//!   1. **Paragraph** boundaries (blank line) — pack whole paragraphs that fit.
//!   2. **Sentence** boundaries (`. ` / `。` / `! ` / `? `) when one paragraph is
//!      itself larger than the target.
//!   3. **Fixed token window** with `overlap`-token carry-over for any single run
//!      still over the limit (e.g. a wall of text with no punctuation).
//!
//! At most [`MAX_CHUNKS`] chunks are produced; the rest is dropped (F_B3 §4.2).
//!
//! Token counting is a whitespace word estimate (~1.3 tokens/word for real BPE),
//! which is accurate enough at the v0.4 target of 200–400 token chunks and avoids
//! loading the tokenizer in the hot path (the card sanctions this estimate).

/// Target tokens per chunk (F_B3 §4.2). Well under bge-m3's 8192 context.
pub const TARGET_TOKENS: usize = 400;
/// Overlap (tokens) carried between adjacent fixed-window chunks (F_B3 §4.2).
pub const OVERLAP_TOKENS: usize = 40;
/// Hard cap on chunks per mail (F_B3 §4.2).
pub const MAX_CHUNKS: usize = 50;

/// Chunk `text` with the project defaults ([`TARGET_TOKENS`], [`OVERLAP_TOKENS`]).
pub fn chunk_mail(text: &str) -> Vec<String> {
    chunk_text(text, TARGET_TOKENS, OVERLAP_TOKENS)
}

/// Split `text` into overlapping chunks of at most `max_tokens` words.
///
/// * Empty / whitespace-only input → empty `Vec` (never panics).
/// * Input under the limit → a single chunk holding the trimmed original.
pub fn chunk_text(text: &str, max_tokens: usize, overlap: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let max_tokens = max_tokens.max(1);
    let overlap = overlap.min(max_tokens.saturating_sub(1));

    // Fast path: whole text fits in one chunk.
    if word_count(trimmed) <= max_tokens {
        return vec![trimmed.to_string()];
    }

    // 1) Break into units no larger than `max_tokens` (paragraph → sentence →
    //    fixed window). Units preserve order; oversized runs become several units.
    let mut units: Vec<String> = Vec::new();
    for para in split_paragraphs(trimmed) {
        if word_count(&para) <= max_tokens {
            units.push(para);
            continue;
        }
        for sentence in split_sentences(&para) {
            if word_count(&sentence) <= max_tokens {
                units.push(sentence);
            } else {
                units.extend(fixed_window(&sentence, max_tokens, overlap));
            }
        }
    }

    // 2) Greedily pack consecutive units that still fit together. Packing keeps
    //    paragraph-sized chunks whole; overlap already lives inside fixed-window
    //    units, so we don't double-apply it here.
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for unit in units {
        if current.is_empty() {
            current = unit;
        } else if word_count(&current) + word_count(&unit) <= max_tokens {
            current.push(' ');
            current.push_str(&unit);
        } else {
            chunks.push(std::mem::take(&mut current));
            current = unit;
        }
        if chunks.len() >= MAX_CHUNKS {
            break;
        }
    }
    if !current.is_empty() && chunks.len() < MAX_CHUNKS {
        chunks.push(current);
    }
    chunks.truncate(MAX_CHUNKS);
    chunks
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Split on blank lines (one or more), trimming each paragraph.
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !buf.is_empty() {
                out.push(buf.join(" ").trim().to_string());
                buf.clear();
            }
        } else {
            buf.push(line.trim());
        }
    }
    if !buf.is_empty() {
        out.push(buf.join(" ").trim().to_string());
    }
    if out.is_empty() {
        out.push(text.trim().to_string());
    }
    out
}

/// Split a paragraph into sentences on `.`, `。`, `!`, `?` boundaries, keeping the
/// terminator attached to its sentence.
fn split_sentences(para: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes: Vec<char> = para.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        let is_terminator = matches!(c, '.' | '!' | '?' | '。' | '！' | '？');
        let next_is_space = bytes.get(i + 1).map(|n| n.is_whitespace()).unwrap_or(true);
        if is_terminator && next_is_space {
            let s: String = bytes[start..=i].iter().collect();
            let s = s.trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            start = i + 1;
        }
    }
    if start < bytes.len() {
        let s: String = bytes[start..].iter().collect();
        let s = s.trim().to_string();
        if !s.is_empty() {
            out.push(s);
        }
    }
    if out.is_empty() {
        out.push(para.trim().to_string());
    }
    out
}

/// Slide a `max_tokens`-word window over `text`, stepping `max_tokens - overlap`
/// words each time so adjacent windows share exactly `overlap` words.
fn fixed_window(text: &str, max_tokens: usize, overlap: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_tokens {
        return vec![words.join(" ")];
    }
    let step = max_tokens.saturating_sub(overlap).max(1);
    let mut out = Vec::new();
    let mut start = 0;
    while start < words.len() {
        let end = (start + max_tokens).min(words.len());
        out.push(words[start..end].join(" "));
        if end == words.len() {
            break;
        }
        start += step;
        if out.len() >= MAX_CHUNKS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_no_chunks() {
        assert!(chunk_text("", 400, 40).is_empty());
        assert!(chunk_text("   \n\n  ", 400, 40).is_empty());
    }

    #[test]
    fn short_text_is_one_complete_chunk() {
        let t = "The quarterly report is attached for your review.";
        let chunks = chunk_text(t, 400, 40);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], t);
    }

    #[test]
    fn long_text_respects_max_and_chunk_cap() {
        // 5000 words, no punctuation → forces the fixed-window path.
        let big = vec!["lorem"; 5000].join(" ");
        let chunks = chunk_text(&big, 400, 40);
        assert!(!chunks.is_empty());
        assert!(chunks.len() <= MAX_CHUNKS, "must respect the 50-chunk cap");
        for c in &chunks {
            assert!(word_count(c) <= 400, "every chunk <= max_tokens");
        }
    }

    #[test]
    fn fixed_window_overlap_is_exact() {
        // 1000 distinct words → windows of 400 step 360 share 40 words.
        let words: Vec<String> = (0..1000).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        let chunks = chunk_text(&text, 400, 40);
        assert!(chunks.len() >= 2);
        let first: Vec<&str> = chunks[0].split_whitespace().collect();
        let second: Vec<&str> = chunks[1].split_whitespace().collect();
        // Last 40 of chunk 0 == first 40 of chunk 1.
        assert_eq!(&first[first.len() - 40..], &second[..40]);
    }

    #[test]
    fn paragraphs_are_kept_whole_when_they_fit() {
        let text = "First short paragraph here.\n\nSecond short paragraph here.";
        let chunks = chunk_text(text, 400, 40);
        // Both fit in one packed chunk.
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("First"));
        assert!(chunks[0].contains("Second"));
    }

    #[test]
    fn sentence_split_used_for_oversized_paragraph() {
        // One paragraph of 12 sentences, max 5 tokens → splits on sentences and
        // packs them; stays within the cap and the token budget.
        let para = (0..12)
            .map(|i| format!("alpha beta gamma s{i}."))
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = chunk_text(&para, 5, 1);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(word_count(c) <= 5);
        }
    }
}
