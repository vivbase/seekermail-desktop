//! Office Open XML + spreadsheet extraction (T108, A5 deepening).
//!
//! * **.docx / .pptx** — Office Open XML is a ZIP of XML parts. We open the
//!   archive (`zip`), read the relevant part(s), and strip tags to recover the
//!   text nodes. This is far lighter than pulling a full DOCX object model and is
//!   sufficient for keyword + semantic indexing (we only need the words).
//! * **.xlsx / .xls** — parsed with `calamine`; every sheet's cells are joined
//!   `\t` per column and `\n` per row, sheets separated by a blank line.
//!
//! The legacy binary `.doc` (OLE2) format is intentionally NOT handled here
//! (F_A5 §9 / T108 §4 Out of Scope) — callers route it to `skipped`.

use std::io::{Cursor, Read};

use crate::error::{AppError, AppResult};

/// Extract text from a `.docx` (Office Open XML word processing) blob.
pub fn extract_docx(bytes: &[u8]) -> AppResult<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("docx open: {e}")))?;
    let xml = read_zip_entry(&mut archive, "word/document.xml")?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("docx: missing word/document.xml")))?;
    Ok(xml_to_text(&xml))
}

/// Extract text from a `.pptx` blob: every `ppt/slides/slideN.xml` in slide order.
pub fn extract_pptx(bytes: &[u8]) -> AppResult<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("pptx open: {e}")))?;

    // Collect slide part names first (immutable borrow), then read each.
    let mut slides: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .map(|n| n.to_string())
        .collect();
    slides.sort_by_key(|name| slide_index(name));

    let mut out = String::new();
    for name in slides {
        if let Some(xml) = read_zip_entry(&mut archive, &name)? {
            let text = xml_to_text(&xml);
            if !text.trim().is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(text.trim());
            }
        }
    }
    Ok(out)
}

/// Extract text from a spreadsheet (`.xlsx` / `.xls` / `.xlsm` / `.ods`) blob via
/// `calamine`. Cells are tab-separated within a row, rows newline-separated, and
/// sheets separated by a blank line.
pub fn extract_spreadsheet(bytes: &[u8]) -> AppResult<String> {
    use calamine::{open_workbook_auto_from_rs, DataType, Reader};

    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("spreadsheet open: {e}")))?;

    let mut out = String::new();
    for sheet in workbook.sheet_names() {
        let range = match workbook.worksheet_range(&sheet) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(sheet = %sheet, error = %e, "spreadsheet: sheet read failed");
                continue;
            }
        };
        if range.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        for row in range.rows() {
            let line: Vec<String> = row
                .iter()
                .map(|c| {
                    if c.is_empty() {
                        String::new()
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            out.push_str(&line.join("\t"));
            out.push('\n');
        }
    }
    Ok(out)
}

/// Read one ZIP entry to a (lossy-UTF-8) string. `Ok(None)` if the entry is absent.
fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> AppResult<Option<String>> {
    let mut file = match archive.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(AppError::Internal(anyhow::anyhow!("zip entry {name}: {e}"))),
    };
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("zip read {name}: {e}")))?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// `ppt/slides/slide12.xml` → 12 (for slide ordering; non-numeric → large).
fn slide_index(name: &str) -> u32 {
    name.trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(u32::MAX)
}

/// Turn an Office Open XML part into plain text: convert paragraph / line-break
/// elements to newlines, drop every other tag, then decode the basic XML
/// entities. Pure and deterministic.
fn xml_to_text(xml: &str) -> String {
    let with_breaks = xml
        .replace("</w:p>", "\n")
        .replace("</a:p>", "\n")
        .replace("</text:p>", "\n")
        .replace("<w:br/>", "\n")
        .replace("<w:br />", "\n")
        .replace("<a:br/>", "\n")
        .replace("<a:br />", "\n");
    let stripped = strip_tags(&with_breaks);
    collapse_blank_lines(&decode_entities(&stripped))
}

/// Remove everything between `<` and `>` (inclusive), keeping text nodes.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Decode the five predefined XML entities (the only ones OOXML text nodes use).
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Collapse runs of 3+ newlines down to 2 and trim trailing spaces per line.
fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_to_text_strips_tags_and_breaks_paragraphs() {
        let xml = "<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p>";
        let text = xml_to_text(xml);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(
            text.contains('\n'),
            "paragraphs should be newline separated"
        );
    }

    #[test]
    fn decode_entities_handles_amp() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt;"), "a & b <c>");
    }

    #[test]
    fn slide_index_orders_numerically() {
        assert!(slide_index("ppt/slides/slide2.xml") < slide_index("ppt/slides/slide10.xml"));
    }

    #[test]
    fn docx_extracts_known_paragraph() {
        // Build a minimal valid .docx (ZIP with word/document.xml) in memory.
        let mut buf = Vec::new();
        {
            use std::io::Write;
            let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
            zw.start_file("word/document.xml", opts).unwrap();
            zw.write_all(
                b"<w:document><w:body><w:p><w:r><w:t>Quarterly budget report</w:t></w:r></w:p></w:body></w:document>",
            )
            .unwrap();
            zw.finish().unwrap();
        }
        let text = extract_docx(&buf).unwrap();
        assert!(text.contains("Quarterly budget report"), "got: {text:?}");
    }
}
