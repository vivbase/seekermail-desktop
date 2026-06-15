//! Plain-text family extraction (T108, A5 deepening).
//!
//! Plain text, Markdown, CSV, JSON, XML, YAML and HTML are decoded as UTF-8
//! (lossy, so a stray non-UTF-8 byte never fails the whole file) and returned
//! as-is. Oversized inputs are capped here at [`PLAINTEXT_READ_CAP`] before the
//! shared 200 KB store-side truncation in [`super`] applies — a defence-in-depth
//! limit so a multi-megabyte log never lands fully in memory.

use crate::error::AppResult;

/// Read cap for the plain-text family: 1 MB. Anything past this is dropped with a
/// WARN; the indexable head is still extracted (F_C1/F_C2 only need the lede for
/// recall, and the 200 KB store cap trims further).
pub const PLAINTEXT_READ_CAP: usize = 1024 * 1024;

/// Decode a plain-text-family blob to a string. Never fails on bad bytes
/// (lossy UTF-8), never panics.
pub fn extract_plaintext(bytes: &[u8]) -> AppResult<String> {
    let slice = if bytes.len() > PLAINTEXT_READ_CAP {
        tracing::warn!(
            len = bytes.len(),
            cap = PLAINTEXT_READ_CAP,
            "plain-text attachment exceeds read cap; truncating head"
        );
        &bytes[..PLAINTEXT_READ_CAP]
    } else {
        bytes
    };
    Ok(String::from_utf8_lossy(slice).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_text() {
        let out = extract_plaintext(b"hello\nworld").unwrap();
        assert_eq!(out, "hello\nworld");
    }

    #[test]
    fn lossy_on_bad_bytes_never_fails() {
        let out = extract_plaintext(&[0xff, 0xfe, b'h', b'i']).unwrap();
        assert!(out.contains("hi"));
    }

    #[test]
    fn caps_oversized_input() {
        let big = vec![b'a'; PLAINTEXT_READ_CAP + 10_000];
        let out = extract_plaintext(&big).unwrap();
        assert_eq!(out.len(), PLAINTEXT_READ_CAP);
    }
}
