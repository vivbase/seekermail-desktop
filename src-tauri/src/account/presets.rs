//! Built-in provider autoconfig presets (T014).
//!
//! Static, compiled-in table mapping an email domain to IMAP/SMTP host/port/TLS.
//! No runtime file reads (T014 §6). `autodiscover` returns `None` for unknown
//! domains, which the wizard (T017) reads as "show the manual server fields".

use crate::types::ProviderHints;

/// One preset row.
struct Preset {
    domains: &'static [&'static str],
    imap_host: &'static str,
    imap_port: u16,
    smtp_host: &'static str,
    smtp_port: u16,
}

/// The preset table. TLS is implied: IMAP 993 / SMTP 465 are implicit TLS, 587 is
/// STARTTLS — every row here uses 993 + 587, so `imap_tls=true`, `smtp_tls=true`
/// (STARTTLS is still "TLS" from the caller's point of view).
const PRESETS: &[Preset] = &[
    Preset {
        domains: &["gmail.com", "googlemail.com"],
        imap_host: "imap.gmail.com",
        imap_port: 993,
        smtp_host: "smtp.gmail.com",
        smtp_port: 587,
    },
    Preset {
        domains: &["outlook.com", "hotmail.com", "live.com", "msn.com"],
        imap_host: "outlook.office365.com",
        imap_port: 993,
        smtp_host: "smtp.office365.com",
        smtp_port: 587,
    },
    Preset {
        domains: &["yahoo.com", "ymail.com"],
        imap_host: "imap.mail.yahoo.com",
        imap_port: 993,
        smtp_host: "smtp.mail.yahoo.com",
        smtp_port: 587,
    },
    Preset {
        domains: &["icloud.com", "me.com", "mac.com"],
        imap_host: "imap.mail.me.com",
        imap_port: 993,
        smtp_host: "smtp.mail.me.com",
        smtp_port: 587,
    },
    Preset {
        domains: &["qq.com"],
        imap_host: "imap.qq.com",
        imap_port: 993,
        smtp_host: "smtp.qq.com",
        smtp_port: 587,
    },
    Preset {
        domains: &["163.com"],
        imap_host: "imap.163.com",
        imap_port: 993,
        smtp_host: "smtp.163.com",
        smtp_port: 465,
    },
    Preset {
        domains: &["126.com"],
        imap_host: "imap.126.com",
        imap_port: 993,
        smtp_host: "smtp.126.com",
        smtp_port: 465,
    },
    Preset {
        domains: &["protonmail.com", "proton.me", "pm.me"],
        // Proton requires the local Bridge; these are the Bridge's loopback ports.
        imap_host: "127.0.0.1",
        imap_port: 1143,
        smtp_host: "127.0.0.1",
        smtp_port: 1025,
    },
];

/// The domain part of an email address, lowercased.
fn domain_of(email: &str) -> Option<String> {
    email.rsplit('@').next().map(|d| d.trim().to_lowercase())
}

/// Look up provider hints by email domain. `None` when no preset matches.
pub fn autodiscover(email: &str) -> Option<ProviderHints> {
    let domain = domain_of(email)?;
    PRESETS
        .iter()
        .find(|p| p.domains.contains(&domain.as_str()))
        .map(|p| ProviderHints {
            imap_host: p.imap_host.to_string(),
            imap_port: p.imap_port,
            imap_tls: true,
            smtp_host: p.smtp_host.to_string(),
            smtp_port: p.smtp_port,
            smtp_tls: true,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_domain_returns_hints() {
        let h = autodiscover("user@gmail.com").unwrap();
        assert_eq!(h.imap_host, "imap.gmail.com");
        assert_eq!(h.imap_port, 993);
        assert!(h.imap_tls);
    }

    #[test]
    fn match_is_case_insensitive() {
        assert!(autodiscover("User@Gmail.COM").is_some());
        assert!(autodiscover("a@GoogleMail.com").is_some());
    }

    #[test]
    fn unknown_domain_returns_none() {
        assert!(autodiscover("user@custom-domain.io").is_none());
    }
}
