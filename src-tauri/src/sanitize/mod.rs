//! HTML sanitisation at ingest (T027, B1/B2 first pass).
//!
//! `Sanitizer::clean` is pure + sync + I/O-free, built once at boot and shared via
//! `Arc` (call it from `spawn_blocking` in the async ingest path, 03 §15). It runs
//! ammonia with a strict allowlist, neutralises remote images (moving the URL to
//! `data-remote-src` so the webview makes zero requests), counts tracker pixels
//! against a bundled rule set, and derives a plain-text body.
//!
//! Defence-in-depth: this is the FIRST pass; the frontend runs DOMPurify again
//! before injection (T028).

use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::{Regex, RegexSet};
use serde::Deserialize;

/// Output of [`Sanitizer::clean`] (03 §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizeOutput {
    /// Allowlisted, remote-image-neutralised HTML, safe to persist in `body_html`.
    pub html: String,
    /// Tag-stripped plain text for `body_text` / FTS.
    pub body_text: String,
    /// Number of tracker pixels detected and blocked.
    pub tracker_count: u32,
    /// Distinct hosts whose remote requests were blocked (domains only — no full
    /// URLs leave this struct, log-safety 09 §5).
    pub blocked_hosts: Vec<String>,
}

#[derive(Deserialize)]
struct SizeThresholds {
    max_tracker_px: u32,
}

#[derive(Deserialize)]
struct RuleSet {
    size_thresholds: SizeThresholds,
    known_trackers: Vec<String>,
    pattern_rules: Vec<String>,
}

/// Rule set compiled in at build time (T027 §6 — `include_str!`, no runtime read).
static RULES_JSON: &str = include_str!("tracker_rules.json");

static IMG_TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<img\b[^>]*>").unwrap());
static SRC_ATTR: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)\bsrc\s*=\s*"([^"]*)""#).unwrap());
static WIDTH_ATTR: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)\bwidth\s*=\s*"?(\d+)"#).unwrap());
static HEIGHT_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\bheight\s*=\s*"?(\d+)"#).unwrap());

/// Matches a `style="…"` attribute (incl. its leading whitespace) in the
/// ammonia-cleaned HTML so we can rewrite the value through [`scrub_style`].
static STYLE_ATTR: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?is)\s+style\s*=\s*"([^"]*)""#).unwrap());

/// A declaration value matching this is dropped: remote loads, script execution,
/// stylesheet imports, or attempts to break out via markup / escapes / entities.
static UNSAFE_CSS_VALUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)url\s*\(|expression\s*\(|javascript:|vbscript:|@import|[<>\\]|/\*|&#"#)
        .unwrap()
});

/// Inert CSS properties that cannot load remote resources or escape the reading
/// pane. Mirrors `SAFE_CSS_PROPERTIES` in the frontend `cssScrub.ts`.
static SAFE_CSS_PROPS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        // Colour & typography
        "color",
        "background-color",
        "font",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "font-variant",
        "line-height",
        "letter-spacing",
        "word-spacing",
        "text-align",
        "text-decoration",
        "text-transform",
        "text-indent",
        "text-overflow",
        "white-space",
        "word-break",
        "overflow-wrap",
        "word-wrap",
        "direction",
        "unicode-bidi",
        "vertical-align",
        // Box model
        "width",
        "min-width",
        "max-width",
        "height",
        "min-height",
        "max-height",
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "padding-block",
        "padding-block-start",
        "padding-block-end",
        "padding-inline",
        "padding-inline-start",
        "padding-inline-end",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "margin-block",
        "margin-block-start",
        "margin-block-end",
        "margin-inline",
        "margin-inline-start",
        "margin-inline-end",
        // Borders
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
        "border-width",
        "border-style",
        "border-color",
        "border-top-width",
        "border-top-style",
        "border-top-color",
        "border-right-width",
        "border-right-style",
        "border-right-color",
        "border-bottom-width",
        "border-bottom-style",
        "border-bottom-color",
        "border-left-width",
        "border-left-style",
        "border-left-color",
        "border-radius",
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-left-radius",
        "border-bottom-right-radius",
        "border-collapse",
        "border-spacing",
        // Table / list / display — positioning props are intentionally absent.
        "display",
        "box-sizing",
        "table-layout",
        "empty-cells",
        "caption-side",
        "list-style-type",
        "list-style-position",
    ]
    .into_iter()
    .collect()
});

/// Allowlisted tags (T027 §4.2).
const ALLOWED_TAGS: &[&str] = &[
    "p",
    "span",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "br",
    "hr",
    "a",
    "img",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
    "blockquote",
    "pre",
    "code",
    "em",
    "strong",
    "b",
    "i",
    "u",
    "sup",
    "sub",
    "figure",
    "figcaption",
];

/// One-time-built HTML sanitiser.
pub struct Sanitizer {
    builder: ammonia::Builder<'static>,
    known: HashSet<String>,
    patterns: RegexSet,
    max_px: u32,
}

impl Default for Sanitizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Sanitizer {
    /// Build the ammonia policy + compile the tracker rules. Done once at boot.
    pub fn new() -> Self {
        let rules: RuleSet =
            serde_json::from_str(RULES_JSON).expect("bundled tracker_rules.json is valid");

        let mut builder = ammonia::Builder::default();
        let tags: HashSet<&str> = ALLOWED_TAGS.iter().copied().collect();
        builder.tags(tags);

        let mut attrs: std::collections::HashMap<&str, HashSet<&str>> =
            std::collections::HashMap::new();
        // NB: do NOT allow `rel` here — ammonia manages it via `link_rel`
        // (default `noopener noreferrer`) and panics if `rel` is also allowlisted.
        attrs.insert("a", ["href", "title", "style"].into_iter().collect());
        attrs.insert(
            "img",
            ["src", "alt", "width", "height", "data-remote-src", "align", "style"]
                .into_iter()
                .collect(),
        );
        // Presentational table attributes keep HTML-mail layout intact. They are
        // inert (no script / URL vectors); `style` is additionally CSS-scrubbed
        // after cleaning (see `scrub_style`). Mirrored in the DOMPurify allowlist
        // (T028) so both passes agree on what layout markup survives.
        attrs.insert(
            "table",
            [
                "width", "height", "align", "bgcolor", "cellpadding", "cellspacing", "border",
                "style",
            ]
            .into_iter()
            .collect(),
        );
        attrs.insert(
            "tr",
            ["align", "valign", "bgcolor", "style"].into_iter().collect(),
        );
        for cell in ["td", "th"] {
            attrs.insert(
                cell,
                [
                    "colspan", "rowspan", "align", "valign", "bgcolor", "width", "height", "style",
                ]
                .into_iter()
                .collect(),
            );
        }
        attrs.insert("p", ["align", "style"].into_iter().collect());
        attrs.insert("div", ["align", "style"].into_iter().collect());
        // Allow a scrubbed `style` on the remaining structural / text tags so
        // inline layout (padding, colour, font sizing) survives ingest.
        for tag in [
            "span", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "pre",
            "code", "em", "strong", "b", "i", "u", "sup", "sub", "figure", "figcaption", "thead",
            "tbody", "hr",
        ] {
            attrs.insert(tag, ["style"].into_iter().collect());
        }
        builder.tag_attributes(attrs);

        // Only safe link/image schemes; `data:` is intentionally excluded so
        // data-URI src injection is dropped (T027 §4.2).
        let schemes: HashSet<&str> = ["http", "https", "mailto", "cid"].into_iter().collect();
        builder.url_schemes(schemes);

        let patterns = RegexSet::new(&rules.pattern_rules).expect("valid pattern rules");
        let known = rules
            .known_trackers
            .into_iter()
            .map(|d| d.to_lowercase())
            .collect();

        Self {
            builder,
            known,
            patterns,
            max_px: rules.size_thresholds.max_tracker_px,
        }
    }

    /// Sanitise raw email HTML. Never fails — malformed HTML is handled by ammonia.
    pub fn clean(&self, raw_html: &str) -> SanitizeOutput {
        let cleaned = self.builder.clean(raw_html).to_string();
        // Keep only inert presentational CSS in inline styles before persisting.
        let styled = scrub_style_attrs(&cleaned);
        let (html, tracker_count, blocked_hosts) = self.neutralise_images(&styled);
        let body_text = html_to_text(&html);
        SanitizeOutput {
            html,
            body_text,
            tracker_count,
            blocked_hosts,
        }
    }

    /// Move every remote `<img src="http…">` to `data-remote-src` (blocking the
    /// request), flag tracker pixels, and collect blocked hosts.
    fn neutralise_images(&self, html: &str) -> (String, u32, Vec<String>) {
        let mut tracker_count = 0u32;
        let mut hosts: Vec<String> = Vec::new();

        let out = IMG_TAG.replace_all(html, |caps: &regex::Captures| {
            let tag = &caps[0];
            let src = SRC_ATTR.captures(tag).map(|c| c[1].to_string());
            let Some(src) = src else {
                return tag.to_string();
            };
            if !(src.starts_with("http://") || src.starts_with("https://")) {
                return tag.to_string(); // relative / cid / already-empty — leave it
            }

            let host = host_of(&src);
            if let Some(h) = &host {
                if !hosts.contains(h) {
                    hosts.push(h.clone());
                }
            }

            let w = WIDTH_ATTR
                .captures(tag)
                .and_then(|c| c[1].parse::<u32>().ok());
            let h = HEIGHT_ATTR
                .captures(tag)
                .and_then(|c| c[1].parse::<u32>().ok());
            let tiny = matches!(w, Some(v) if v <= self.max_px)
                || matches!(h, Some(v) if v <= self.max_px);
            let listed = host
                .as_ref()
                .map(|host| {
                    self.known
                        .iter()
                        .any(|k| host == k || host.ends_with(&format!(".{k}")))
                })
                .unwrap_or(false);
            let pattern = self.patterns.is_match(&src);
            if tiny || listed || pattern {
                tracker_count += 1;
            }

            // Replace the src value with empty + stash the original URL.
            let escaped = src.replace('"', "%22");
            SRC_ATTR
                .replace(
                    tag,
                    format!(r#"src="" data-remote-src="{escaped}""#).as_str(),
                )
                .to_string()
        });

        (out.into_owned(), tracker_count, hosts)
    }
}

/// Extract the lowercase host from an http(s) URL.
fn host_of(url: &str) -> Option<String> {
    let after = url.split("://").nth(1)?;
    let host = after
        .split(['/', '?', '#', ':'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Reduce an inline `style` value to inert presentational declarations only,
/// mirroring the frontend `scrubInlineStyle` (defence-in-depth, 07 §10). Drops
/// any declaration whose property is not allowlisted or whose value can load
/// remote resources / break out of the reading pane. Returns "" when nothing
/// safe remains.
fn scrub_style(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for decl in raw.split(';') {
        if out.len() >= 64 {
            break;
        }
        let Some(idx) = decl.find(':') else { continue };
        let prop = decl[..idx].trim().to_lowercase();
        let value = decl[idx + 1..].trim();
        if prop.is_empty() || value.is_empty() || value.len() > 256 {
            continue;
        }
        if !SAFE_CSS_PROPS.contains(prop.as_str()) {
            continue;
        }
        if UNSAFE_CSS_VALUE.is_match(value) {
            continue;
        }
        out.push(format!("{prop}: {value}"));
    }
    out.join("; ")
}

/// Rewrite every `style="…"` in cleaned HTML through [`scrub_style`], dropping
/// the attribute entirely when nothing safe survives.
fn scrub_style_attrs(html: &str) -> String {
    STYLE_ATTR
        .replace_all(html, |caps: &regex::Captures| {
            let cleaned = scrub_style(&caps[1]);
            if cleaned.is_empty() {
                String::new()
            } else {
                format!(r#" style="{cleaned}""#)
            }
        })
        .into_owned()
}

/// Convert sanitised HTML to readable plain text: block tags → newlines, strip
/// remaining tags, decode the common entities, collapse runs of blank lines.
pub fn html_to_text(html: &str) -> String {
    static BLOCK_BREAK: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?is)</(p|div|tr|li|h[1-6]|blockquote|figure|figcaption)\s*>|<br\s*/?>|</?(table|thead|tbody|ul|ol)\s*>")
            .unwrap()
    });
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<[^>]+>").unwrap());
    static MANY_NL: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());

    let with_breaks = BLOCK_BREAK.replace_all(html, "\n");
    let stripped = TAG.replace_all(&with_breaks, "");
    let decoded = decode_entities(&stripped);
    let collapsed = MANY_NL.replace_all(&decoded, "\n\n");
    collapsed
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Sanitizer {
        Sanitizer::new()
    }

    #[test]
    fn strips_scripts_and_handlers() {
        let out = s().clean(r#"<script>alert(1)</script><p onclick="x()">Hi</p>"#);
        assert!(!out.html.contains("script"));
        assert!(!out.html.contains("alert"));
        assert!(!out.html.contains("onclick"));
        assert!(out.html.contains("Hi"));
    }

    #[test]
    fn drops_iframe_and_data_uri_img() {
        let out =
            s().clean(r#"<iframe src="evil"></iframe><img src="data:image/png;base64,AAAA">"#);
        assert!(!out.html.contains("iframe"));
        // data: src is removed by ammonia (scheme not allowed).
        assert!(!out.html.contains("data:image"));
    }

    #[test]
    fn keeps_allowed_structure() {
        let html = r#"<table><tr><td colspan="2">cell</td></tr></table><blockquote>q</blockquote><a href="https://x.com">l</a>"#;
        let out = s().clean(html);
        assert!(out.html.contains("<table"));
        assert!(out.html.contains("<blockquote"));
        assert!(out.html.contains("href=\"https://x.com\""));
    }

    #[test]
    fn remote_image_moved_to_data_remote_src() {
        let out = s().clean(r#"<img src="https://cdn.example.com/a.png" alt="pic">"#);
        assert!(out.html.contains(r#"src="""#));
        assert!(out
            .html
            .contains(r#"data-remote-src="https://cdn.example.com/a.png""#));
        assert!(out.blocked_hosts.contains(&"cdn.example.com".to_string()));
    }

    #[test]
    fn tracker_pixel_is_counted() {
        // 1x1 image → tracker.
        let out = s().clean(r#"<img src="https://track.example.com/p.gif" width="1" height="1">"#);
        assert_eq!(out.tracker_count, 1);
        // Known tracker domain → tracker even without tiny size.
        let out2 = s().clean(r#"<img src="https://x.list-manage.com/open?id=1" width="600">"#);
        assert_eq!(out2.tracker_count, 1);
    }

    #[test]
    fn body_text_has_no_tags_and_keeps_breaks() {
        let out = s().clean("<p>First</p><p>Second</p>");
        assert!(!out.body_text.contains('<'));
        assert!(out.body_text.contains("First"));
        assert!(out.body_text.contains("Second"));
        assert!(out.body_text.contains('\n'));
    }

    #[test]
    fn javascript_href_is_stripped() {
        let out = s().clean(r#"<a href="javascript:alert(1)">x</a>"#);
        assert!(!out.html.contains("javascript:"));
    }

    #[test]
    fn keeps_presentational_layout_attributes() {
        let html = r#"<table width="600" bgcolor="#ffffff" align="center" cellpadding="0" cellspacing="0"><tr valign="top"><td align="left" width="50%">cell</td></tr></table>"#;
        let out = s().clean(html);
        assert!(out.html.contains(r#"width="600""#));
        assert!(out.html.contains(r#"bgcolor="#ffffff""#));
        assert!(out.html.contains(r#"align="center""#));
        assert!(out.html.contains(r#"valign="top""#));
        assert!(out.html.contains(r#"align="left""#));
    }

    #[test]
    fn keeps_safe_inline_style_and_drops_dangerous() {
        let out = s().clean(
            r#"<p style="color:#333; padding:10px; background-color:url(http://t.example/x.png); position:fixed">hi</p>"#,
        );
        // Safe presentational declarations survive…
        assert!(out.html.contains("color: #333"));
        assert!(out.html.contains("padding: 10px"));
        // …url() value is dropped and positioning is not allowlisted at all.
        assert!(!out.html.contains("url("));
        assert!(!out.html.to_lowercase().contains("position"));
    }

    #[test]
    fn drops_css_expression_and_import() {
        let out = s().clean(
            r#"<div style="width:expression(alert(1)); color:red"><span style="@import 'evil.css'; font-size:12px">x</span></div>"#,
        );
        assert!(out.html.contains("color: red"));
        assert!(out.html.contains("font-size: 12px"));
        assert!(!out.html.to_lowercase().contains("expression"));
        assert!(!out.html.contains("@import"));
    }

    #[test]
    fn drops_style_attribute_when_nothing_safe_remains() {
        let out = s().clean(r#"<p style="position:absolute; behavior:url(x.htc)">body</p>"#);
        assert!(out.html.contains("body"));
        assert!(!out.html.contains("style="));
    }
}
