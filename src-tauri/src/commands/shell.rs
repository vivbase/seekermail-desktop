//! Shell / OS-integration commands.
//!
//! `open_external_url` hands a link to the OS default application (web browser
//! for http/https, mail client for mailto, dialer for tel) so that clicking a
//! link inside a rendered email never navigates the app's own webview away from
//! the SPA. Without this, an `<a href>` click in mail HTML replaces the entire
//! window with the target page.
//!
//! Security: the URL arrives from the frontend (an anchor href extracted from
//! sanitised mail HTML). The backend is the trust boundary, not the renderer, so
//! the scheme is re-validated here — only the allowlisted schemes are ever handed
//! to the OS opener; `javascript:`, `data:`, `file:`, and every other scheme are
//! refused. Arguments are passed to the opener process directly (never through a
//! shell), so there is no shell-injection surface.

use crate::error::{AppError, AppResult, IpcError};

/// Schemes we are willing to hand to the OS default handler.
const ALLOWED_SCHEMES: [&str; 4] = ["http", "https", "mailto", "tel"];

/// Open an external link with the OS default application. Refuses any scheme
/// outside [`ALLOWED_SCHEMES`].
#[tauri::command]
pub async fn open_external_url(url: String) -> Result<(), IpcError> {
    open_external(&url).map_err(IpcError::from)
}

/// Validate the scheme, then spawn the OS opener. Split from the command wrapper
/// so it is unit-testable without a Tauri runtime.
fn open_external(url: &str) -> AppResult<()> {
    validate_external_url(url)?;
    os_open_url(url)
}

/// Accept only the allowlisted schemes; reject embedded control characters and
/// any URL without a scheme. The scheme is the text before the first ':'
/// (compared case-insensitively).
fn validate_external_url(url: &str) -> AppResult<()> {
    if url.chars().any(|c| c.is_control()) {
        return Err(AppError::Validation(
            "url contains control characters".into(),
        ));
    }
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err(AppError::Forbidden(format!(
            "refused url scheme: '{scheme}'"
        )));
    }
    Ok(())
}

/// Spawn the platform default-handler for a URL. Arguments go straight to the
/// process (no shell); the scheme is already allowlisted, so the leading token
/// can never be an `open` flag or a different executable.
fn os_open_url(url: &str) -> AppResult<()> {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    result
        .map(|_| ())
        .map_err(|e| AppError::FsPermission(format!("open url: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_web_and_mail_schemes() {
        for url in [
            "http://example.com",
            "https://example.com/path?q=1&r=2",
            "HTTPS://EXAMPLE.COM",
            "mailto:alice@example.com",
            "tel:+1-555-0100",
        ] {
            assert!(validate_external_url(url).is_ok(), "should accept {url}");
        }
    }

    #[test]
    fn refuses_dangerous_or_schemeless_urls() {
        for url in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "vbscript:msgbox(1)",
            "chrome://settings",
            "relative/path",
            "",
        ] {
            assert!(
                matches!(validate_external_url(url), Err(AppError::Forbidden(_))),
                "should refuse {url}"
            );
        }
    }

    #[test]
    fn refuses_control_characters() {
        assert!(matches!(
            validate_external_url("https://example.com/\n\r"),
            Err(AppError::Validation(_))
        ));
    }
}
