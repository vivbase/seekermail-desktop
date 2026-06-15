//! PDF text extraction (T108, A5 deepening).
//!
//! Uses the pure-Rust `pdf-extract` crate (no native libs, no OCR). A PDF whose
//! extractable body is shorter than [`MIN_PDF_TEXT_BYTES`] is almost certainly a
//! scanned image with no text layer; rather than ship an OCR engine (explicitly
//! out of scope, F_A5 §9) we return [`PdfOutcome::NoText`] so the caller marks the
//! row `skipped` with a stable sentinel.

use crate::error::{AppError, AppResult};

/// Below this many extracted bytes a PDF is treated as a scan with no text layer.
pub const MIN_PDF_TEXT_BYTES: usize = 100;

/// Sentinel stored in `extracted_text` for image-only PDFs (no OCR).
pub const NO_TEXT_SENTINEL: &str = "[PDF: no extractable text]";

/// Result of a PDF extraction attempt.
pub enum PdfOutcome {
    /// Real text was recovered.
    Text(String),
    /// The PDF parsed but yielded no usable text layer (scanned image).
    NoText,
}

/// Extract text from PDF bytes. Returns [`PdfOutcome::NoText`] for scans, or an
/// [`AppError`] only for genuinely corrupt / unreadable files. `pdf-extract` can
/// panic on malformed input, so the caller wraps this in `catch_unwind`.
pub fn extract_pdf(bytes: &[u8]) -> AppResult<PdfOutcome> {
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("pdf parse: {e}")))?;
    let trimmed = text.trim();
    if trimmed.len() < MIN_PDF_TEXT_BYTES {
        Ok(PdfOutcome::NoText)
    } else {
        Ok(PdfOutcome::Text(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_pdf_is_error_not_panic() {
        // Not a PDF at all — must surface as an error (caught by the caller), and
        // must not panic the test process.
        let res = std::panic::catch_unwind(|| extract_pdf(b"%PDF-1.4 not really a pdf"));
        assert!(res.is_ok(), "extract_pdf must not unwind on junk input");
        assert!(res.unwrap().is_err());
    }
}
