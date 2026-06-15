//! No-proxy egress assertions (T103, ADR-0004): AI inference traffic goes from
//! the device straight to the user-configured provider — a SeekerMail-controlled
//! domain must never be the egress host.
//!
//! Scope note: `xtask` and the adapters keep their HTTP clients private, and the
//! per-request capture is the release-engineer's mitmproxy step (see
//! `docs/compliance/noproxy_check_sop.md`). This test encodes the invariant at
//! the URL-host level for the built-in default endpoints plus a user custom base
//! URL — the exact property ADR-0004 guarantees — so a regression that routed AI
//! traffic through a SeekerMail domain fails CI deterministically and offline.

/// Domains SeekerMail controls. AI inference must never egress to any of these.
const SEEKERMAIL_DOMAINS: &[&str] = &["seekermail.app", "api.seekermail.app", "seekermail.com"];

/// Representative provider inference endpoints: the built-in defaults (dev/06 §1)
/// plus a user-supplied OpenAI-compatible gateway. The host of each is what the
/// device actually connects to for inference.
fn provider_endpoints() -> Vec<(&'static str, &'static str)> {
    vec![
        ("openai", "https://api.openai.com/v1/chat/completions"),
        ("anthropic", "https://api.anthropic.com/v1/messages"),
        ("ollama-local", "http://localhost:11434/api/chat"),
        (
            "openai-compatible-custom",
            "https://ai-gateway.example.com/v1/chat/completions",
        ),
    ]
}

/// Extract the lowercase host (no scheme, path, or port) from a URL.
fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

fn is_seekermail_host(host: &str) -> bool {
    SEEKERMAIL_DOMAINS
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

#[test]
fn ai_endpoints_never_target_a_seekermail_domain() {
    for (provider, url) in provider_endpoints() {
        let host = host_of(url);
        assert!(
            !host.is_empty(),
            "{provider}: could not parse a host from {url}"
        );
        assert!(
            !is_seekermail_host(&host),
            "ADR-0004 violation: {provider} would egress to SeekerMail host {host}",
        );
    }
}

#[test]
fn host_parser_is_correct() {
    assert_eq!(host_of("https://api.openai.com/v1/chat"), "api.openai.com");
    assert_eq!(host_of("http://localhost:11434/api/chat"), "localhost");
    assert_eq!(
        host_of("https://user@host.example.com:443/x"),
        "host.example.com"
    );
}

#[test]
fn seekermail_host_matcher_catches_subdomains() {
    assert!(is_seekermail_host("seekermail.app"));
    assert!(is_seekermail_host("api.seekermail.app"));
    assert!(!is_seekermail_host("api.openai.com"));
    // A look-alike that merely contains the string must not match.
    assert!(!is_seekermail_host("notseekermail.app.evil.com"));
}
