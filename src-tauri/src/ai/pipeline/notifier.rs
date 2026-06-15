//! Throttled OS notification for freshly generated E2 drafts (T083 §3,
//! F_E2 §4.5).
//!
//! One merged notification per batch, at most one per account per
//! [`NOTIFY_THROTTLE_SECS`]. The body carries the *count only* — never a
//! subject, sender, or any mail content (AI_MODES §4.2).
//!
//! The actual OS call is an injected [`NotificationSender`] closure so the
//! throttle logic is unit-testable without an `AppHandle`: `lib.rs` installs
//! the real `tauri-plugin-notification` sender once the app handle exists;
//! until then (and in tests/bootstrap) the notifier is a silent no-op.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::util::now_unix;

/// Minimum spacing between two notifications for the same account (300 s).
pub const NOTIFY_THROTTLE_SECS: i64 = 300;

/// `(title, body)` → platform notification. Installed by `lib.rs` at setup.
pub type NotificationSender = Box<dyn Fn(&str, &str) + Send + Sync>;

struct NotifierInner {
    /// Unix seconds of the last notification per account.
    last_notified: HashMap<String, i64>,
    sender: Option<NotificationSender>,
}

/// Per-account throttled draft notifier. Lives in `AppState` behind an `Arc`.
pub struct DraftNotifier {
    inner: Mutex<NotifierInner>,
}

impl DraftNotifier {
    /// A notifier with no sender installed yet (bootstrap / tests): the
    /// throttle bookkeeping runs, the OS call is skipped.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(NotifierInner {
                last_notified: HashMap::new(),
                sender: None,
            }),
        }
    }

    /// Install (or replace) the platform sender. Called once from `lib.rs`
    /// setup with the `tauri-plugin-notification` closure.
    pub fn set_sender(&self, sender: NotificationSender) {
        self.inner.lock().expect("notifier poisoned").sender = Some(sender);
    }

    /// Notify "N AI draft(s) ready for review" for one account, throttled to
    /// one notification per [`NOTIFY_THROTTLE_SECS`]. Returns whether a
    /// notification was actually emitted (also `false` while throttled).
    pub fn notify_if_needed(&self, account_id: &str, draft_count: usize) -> bool {
        self.notify_if_needed_at(account_id, draft_count, now_unix())
    }

    /// Clock-injected form (the unit-testable core, mirroring the
    /// `ai/fallback.rs` injected-now pattern).
    pub fn notify_if_needed_at(&self, account_id: &str, draft_count: usize, now: i64) -> bool {
        if draft_count == 0 {
            return false;
        }
        let mut inner = self.inner.lock().expect("notifier poisoned");
        if let Some(last) = inner.last_notified.get(account_id) {
            if now - last < NOTIFY_THROTTLE_SECS {
                tracing::debug!(
                    event = "draft_notify_throttled",
                    account_id = %account_id,
                    draft_count = draft_count,
                    "draft notification suppressed by the per-account throttle"
                );
                return false;
            }
        }
        inner.last_notified.insert(account_id.to_string(), now);

        // Count only — no subjects, no senders (F_E2 §4.5).
        let body = format!(
            "{draft_count} AI draft{} ready for review",
            if draft_count == 1 { "" } else { "s" }
        );
        if let Some(sender) = &inner.sender {
            sender("SeekerMail", &body);
        }
        tracing::info!(
            event = "draft_notify_sent",
            account_id = %account_id,
            draft_count = draft_count,
            "merged draft notification emitted"
        );
        true
    }
}

impl Default for DraftNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DraftNotifier {
    /// Identifier-only Debug (the sender closure is not introspectable).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DraftNotifier")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    fn counting_notifier() -> (DraftNotifier, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let bodies: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let notifier = DraftNotifier::new();
        let c = calls.clone();
        let b = bodies.clone();
        notifier.set_sender(Box::new(move |title: &str, body: &str| {
            assert_eq!(title, "SeekerMail");
            c.fetch_add(1, Ordering::SeqCst);
            b.lock().unwrap().push(body.to_string());
        }));
        (notifier, calls, bodies)
    }

    #[test]
    fn first_call_notifies_second_is_throttled_third_passes_after_window() {
        let (notifier, calls, _) = counting_notifier();
        let t0 = 1_750_000_000;
        assert!(notifier.notify_if_needed_at("acc", 3, t0));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Inside the 300 s window → suppressed.
        assert!(!notifier.notify_if_needed_at("acc", 2, t0 + 200));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Past the window → emitted again.
        assert!(notifier.notify_if_needed_at("acc", 1, t0 + NOTIFY_THROTTLE_SECS));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn throttle_is_per_account() {
        let (notifier, calls, _) = counting_notifier();
        let t0 = 1_750_000_000;
        assert!(notifier.notify_if_needed_at("a", 1, t0));
        assert!(notifier.notify_if_needed_at("b", 1, t0));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn body_is_count_only_with_plural_handling() {
        let (notifier, _, bodies) = counting_notifier();
        notifier.notify_if_needed_at("a", 1, 1_000);
        notifier.notify_if_needed_at("b", 4, 1_000);
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies[0], "1 AI draft ready for review");
        assert_eq!(bodies[1], "4 AI drafts ready for review");
    }

    #[test]
    fn zero_count_and_missing_sender_are_no_ops() {
        let (notifier, calls, _) = counting_notifier();
        assert!(!notifier.notify_if_needed_at("a", 0, 1_000));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // No sender installed: throttle still tracks, nothing panics.
        let silent = DraftNotifier::new();
        assert!(silent.notify_if_needed_at("a", 2, 1_000));
        assert!(!silent.notify_if_needed_at("a", 2, 1_100));
    }
}
