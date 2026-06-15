//! Log-safety assertions (T103, dev/09 §5): secret-bearing values must never
//! reach `tracing` output or a `Debug` rendering. We capture events with a
//! minimal in-process subscriber (no extra deps) and assert a denylist of canary
//! secrets never appears, plus a positive control so the assertions aren't
//! trivially passing on empty output. We also assert the two key-bearing param
//! types redact their `Debug` (the real leak surface, dev/09 §5).

use seekermail_lib::error::{AppError, IpcError};
use seekermail_lib::types::{AiProvider, UpdateAiSettingsParams, VerifyAiProviderParams};

/// Canary secrets that must never appear anywhere in logs (one per denied class,
/// dev/09 §5: AI key, mailbox password, mail body, OAuth token, prompt text).
const DENYLIST: &[&str] = &[
    "supersecret-api-key-zzz",
    "hunter2-imap-password",
    "Dear team, please wire the amount",
    "ya29.oauth-access-token-xyz",
    "PROMPT: summarise the following private email",
];

/// A minimal `tracing::Subscriber` that records each event's fields into a shared
/// buffer — enough to assert on emitted log content without `tracing-subscriber`.
mod capture {
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    #[derive(Clone, Default)]
    pub struct Capture {
        lines: Arc<Mutex<Vec<String>>>,
    }

    impl Capture {
        /// A clone sharing the same buffer — pass to `with_default`, read via `self`.
        pub fn handle(&self) -> Self {
            self.clone()
        }
        pub fn lines(&self) -> Vec<String> {
            self.lines.lock().expect("capture lock").clone()
        }
    }

    struct LineVisitor(String);
    impl Visit for LineVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.push_str(&format!(" {}={value:?}", field.name()));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push_str(&format!(" {}={value}", field.name()));
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            let mut v = LineVisitor(format!("[{}]", event.metadata().level()));
            event.record(&mut v);
            self.lines.lock().expect("capture lock").push(v.0);
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }
}

#[test]
fn ipc_boundary_log_carries_code_not_secrets() {
    let cap = capture::Capture::default();
    tracing::subscriber::with_default(cap.handle(), || {
        // `From<AppError> for IpcError` is the single boundary log point (dev/09 §3).
        let _: IpcError = AppError::AuthInvalidCredentials.into();
        let _: IpcError = AppError::AuthKeychainDenied.into();
        let _: IpcError = AppError::Validation("auth_level must be 1, 2, or 3".into()).into();
        let _: IpcError = AppError::Forbidden("cannot delete the last account".into()).into();
    });

    let blob = cap.lines().join("\n");
    assert!(
        !blob.is_empty(),
        "the boundary must emit at least one log line"
    );
    // The stable wire code is present (so logging works at all)...
    assert!(
        blob.contains("AUTH_INVALID_CREDENTIALS"),
        "expected code in log: {blob}"
    );
    // ...and not a single canary secret leaked through any field.
    for secret in DENYLIST {
        assert!(!blob.contains(secret), "secret leaked into logs: {secret}");
    }
}

#[test]
fn update_ai_settings_debug_redacts_the_api_key() {
    let params = UpdateAiSettingsParams {
        ai_api_key: Some("supersecret-api-key-zzz".into()),
        ai_model: Some("gpt-4o-mini".into()),
        ..Default::default()
    };
    let rendered = format!("{params:?}");
    assert!(
        !rendered.contains("supersecret-api-key-zzz"),
        "key leaked: {rendered}"
    );
    assert!(
        rendered.contains("***"),
        "key should render as ***: {rendered}"
    );
    // A non-secret field is still visible (the redaction is targeted).
    assert!(rendered.contains("gpt-4o-mini"));
}

#[test]
fn verify_ai_provider_debug_redacts_the_api_key() {
    let params = VerifyAiProviderParams {
        provider: AiProvider::Openai,
        model: "gpt-4o-mini".into(),
        api_key: Some("supersecret-api-key-zzz".into()),
        base_url: None,
    };
    let rendered = format!("{params:?}");
    assert!(
        !rendered.contains("supersecret-api-key-zzz"),
        "key leaked: {rendered}"
    );
    assert!(
        rendered.contains("***"),
        "key should render as ***: {rendered}"
    );
}

#[test]
fn positive_control_benign_content_is_captured() {
    // Guards against the denylist assertions passing on empty/dropped output.
    let cap = capture::Capture::default();
    tracing::subscriber::with_default(cap.handle(), || {
        tracing::error!(error_code = "VALIDATION", "command failed at ipc boundary");
    });
    assert!(cap.lines().join("\n").contains("VALIDATION"));
}
