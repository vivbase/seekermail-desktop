//! Keyword-search DSL parser (T032, F_C1 §4).
//!
//! Parses the user's raw query into a [`DslQuery`]: an FTS5 `MATCH` expression for
//! the indexed columns plus the non-FTS filters (`to:`, `in:`, `has:attachment`)
//! that are applied as SQL `WHERE` clauses. Parsing is **total** — a malformed
//! token degrades to a plain term, never an error (F_C1 §4, T032 §3).
//!
//! Injection safety: every user term is emitted **double-quoted** inside the MATCH
//! string (FTS5 treats a quoted run as a literal string token), and any embedded
//! `"` is doubled per the FTS5 grammar. Non-FTS filter values never touch the
//! MATCH string — they are bound parameters in `fts5.rs`.

/// A parsed keyword query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DslQuery {
    /// FTS5 `MATCH` expression, already escaped. Empty when the query carries only
    /// non-FTS filters (e.g. just `has:attachment`).
    pub fts_match: String,
    /// `to:` — substring match against the JSON `to_addrs` column (not FTS).
    pub to_filter: Option<String>,
    /// `in:` — exact `folder` filter.
    pub folder: Option<String>,
    /// `has:attachment` flag.
    pub has_attachment: bool,
    /// Lower-cased plain terms (for the snippet column heuristic + history).
    pub plain_terms: Vec<String>,
    /// True when a `subject:` field token appeared (snippet prefers col 0).
    pub wants_subject_col: bool,
    /// True when a `from:` field token appeared (snippet prefers cols 2/3).
    pub wants_from_col: bool,
}

/// Parse a raw query string into a [`DslQuery`]. Never panics, never errors.
pub fn parse_keyword_query(raw: &str) -> DslQuery {
    let mut q = DslQuery::default();
    let tokens = tokenize(raw);

    // Build OR-groups of AND-ed FTS expressions, plus a trailing NOT list.
    let mut or_groups: Vec<Vec<String>> = Vec::new();
    let mut and_buf: Vec<String> = Vec::new();
    let mut excludes: Vec<String> = Vec::new();

    for tok in tokens {
        if tok.eq_ignore_ascii_case("OR") {
            if !and_buf.is_empty() {
                or_groups.push(std::mem::take(&mut and_buf));
            }
            continue;
        }
        if tok.eq_ignore_ascii_case("AND") {
            continue; // implicit; ignore the explicit keyword
        }

        // Field-prefixed token? (split on the first ':').
        if let Some((field, value)) = split_field(&tok) {
            let value_trimmed = value.trim_matches('"');
            match field.as_str() {
                "from" => {
                    q.wants_from_col = true;
                    and_buf.push(format!(
                        "{{from_name from_email}} : {}",
                        quote(value_trimmed)
                    ));
                    q.plain_terms.push(value_trimmed.to_lowercase());
                }
                "subject" => {
                    q.wants_subject_col = true;
                    and_buf.push(format!("subject : {}", quote(value_trimmed)));
                    q.plain_terms.push(value_trimmed.to_lowercase());
                }
                "to" => {
                    q.to_filter = Some(value_trimmed.to_lowercase());
                }
                "in" => {
                    q.folder = Some(value_trimmed.to_string());
                }
                "has"
                    if (value_trimmed.eq_ignore_ascii_case("attachment")
                        || value_trimmed.eq_ignore_ascii_case("attachments")) =>
                {
                    q.has_attachment = true;
                }
                // `tag:` reserved → treat the value as a plain term for now (T032 §3).
                _ => {
                    and_buf.push(quote(value_trimmed));
                    q.plain_terms.push(value_trimmed.to_lowercase());
                }
            }
            continue;
        }

        // Exclusion (-term).
        if let Some(stripped) = tok.strip_prefix('-') {
            if !stripped.is_empty() {
                excludes.push(quote(stripped.trim_matches('"')));
            }
            continue;
        }

        // Plain term or quoted phrase.
        let cleaned = tok.trim_matches('"');
        if !cleaned.is_empty() {
            and_buf.push(quote(cleaned));
            q.plain_terms.push(cleaned.to_lowercase());
        }
    }
    if !and_buf.is_empty() {
        or_groups.push(and_buf);
    }

    // Assemble the MATCH string.
    let positive = if or_groups.is_empty() {
        String::new()
    } else if or_groups.len() == 1 {
        or_groups[0].join(" AND ")
    } else {
        or_groups
            .iter()
            .map(|g| format!("({})", g.join(" AND ")))
            .collect::<Vec<_>>()
            .join(" OR ")
    };

    q.fts_match = if positive.is_empty() {
        String::new() // excludes alone can't form a valid FTS query
    } else if excludes.is_empty() {
        positive
    } else {
        let nots = excludes
            .iter()
            .map(|e| format!("NOT {e}"))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{positive} {nots}")
    };

    q
}

/// Split a query into tokens, treating a `"quoted phrase"` (optionally
/// `field:"quoted phrase"`) as a single token.
fn tokenize(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in raw.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                cur.push(c);
            }
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Split `field:value` on the first colon, only when the field is a known prefix
/// (so `http://x` or `12:00` stay plain terms).
fn split_field(tok: &str) -> Option<(String, String)> {
    let (field, value) = tok.split_once(':')?;
    let field_l = field.to_lowercase();
    const FIELDS: [&str; 6] = ["from", "to", "subject", "in", "has", "tag"];
    if FIELDS.contains(&field_l.as_str()) && !value.is_empty() {
        Some((field_l, value.to_string()))
    } else {
        None
    }
}

/// Emit a single FTS5 string token: strip control/quote chars and wrap in double
/// quotes, doubling any internal quote (FTS5 escaping).
fn quote(term: &str) -> String {
    let cleaned: String = term
        .chars()
        .filter(|c| !matches!(c, '\0' | '\r' | '\n'))
        .collect::<String>()
        .replace('"', "\"\"");
    format!("\"{cleaned}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_term_becomes_quoted_match() {
        let q = parse_keyword_query("report");
        assert_eq!(q.fts_match, "\"report\"");
        assert!(!q.has_attachment);
    }

    #[test]
    fn implicit_and() {
        let q = parse_keyword_query("budget report");
        assert_eq!(q.fts_match, "\"budget\" AND \"report\"");
    }

    #[test]
    fn from_field_targets_columns() {
        let q = parse_keyword_query("from:alice@example.com");
        assert_eq!(
            q.fts_match,
            "{from_name from_email} : \"alice@example.com\""
        );
        assert!(q.wants_from_col);
    }

    #[test]
    fn subject_phrase() {
        let q = parse_keyword_query("subject:\"Q4 budget\"");
        assert_eq!(q.fts_match, "subject : \"Q4 budget\"");
        assert!(q.wants_subject_col);
    }

    #[test]
    fn has_attachment_sets_flag_not_match() {
        let q = parse_keyword_query("has:attachment");
        assert!(q.has_attachment);
        assert_eq!(q.fts_match, "");
    }

    #[test]
    fn to_and_in_are_non_fts_filters() {
        let q = parse_keyword_query("to:bob@x.com in:Sent invoice");
        assert_eq!(q.to_filter.as_deref(), Some("bob@x.com"));
        assert_eq!(q.folder.as_deref(), Some("Sent"));
        assert_eq!(q.fts_match, "\"invoice\"");
    }

    #[test]
    fn explicit_or() {
        let q = parse_keyword_query("invoice OR receipt");
        assert_eq!(q.fts_match, "(\"invoice\") OR (\"receipt\")");
    }

    #[test]
    fn exclusion_uses_not() {
        let q = parse_keyword_query("report -draft");
        assert_eq!(q.fts_match, "\"report\" NOT \"draft\"");
    }

    #[test]
    fn injection_chars_are_escaped() {
        let q = parse_keyword_query("foo\" OR 1=1 --");
        // The stray quote is doubled and everything stays inside string tokens.
        assert!(q.fts_match.contains("\"\""));
        assert!(!q.fts_match.contains("1=1\""));
    }

    #[test]
    fn malformed_never_panics() {
        for raw in ["", "   ", "\"", ":::", "from:", "-", "OR OR", "tag:"] {
            let _ = parse_keyword_query(raw); // must not panic
        }
    }
}
