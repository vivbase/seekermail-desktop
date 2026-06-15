//! E2/E3 AI-safety measurement harness (T104, AI_MODES_DESIGN §4.3/§5/§11).
//!
//! Runs a labelled fixture mail set through the auto-reply safety rules and
//! reports the **full-auto misfire rate** (mails that should have been
//! demoted/skipped but would auto-send) and the **sensitive-downgrade rate**
//! (mails correctly demoted by the E4 forced rules). Mock-only: no network, no
//! API spend.
//!
//! Reconciliation note: the card asks the runner to call the production
//! `sensitive_pre_scan` / `pre_send_check`. `xtask` is *deliberately* not a
//! member of the app workspace (see `xtask/Cargo.toml` — it must not perturb the
//! shipping crate's dependency / supply-chain graph), so it cannot link the
//! `seekermail` crate. Instead [`evaluate`] mirrors the E4 §5 forced-demotion
//! rules (attachment / amount / important-contact are non-disableable) plus the
//! E2 bulk-skip rule, over the fixture metadata. Wiring the live pipeline would
//! require breaking that isolation and is tracked as a follow-up.

pub mod gate;
pub mod runner;
pub mod seed;

use serde::{Deserialize, Serialize};

/// One labelled fixture mail (the JSON shape in `fixtures/initial_set.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub id: String,
    pub subject: String,
    pub body_snippet: String,
    pub sender: String,
    pub has_attachment: bool,
    pub has_amount: bool,
    pub important_contact: bool,
    /// Marketing / pure-notification / CC-only mail the E2 filter should skip.
    #[serde(default)]
    pub is_bulk: bool,
    /// `sent_auto` | `demoted` | `skipped`.
    pub expected_outcome: String,
}

/// The committed fixture set, embedded so `safety-run` needs no prior seed step.
pub const FIXTURES_JSON: &str = include_str!("fixtures/initial_set.json");

pub fn load_fixtures() -> anyhow::Result<Vec<Fixture>> {
    Ok(serde_json::from_str(FIXTURES_JSON)?)
}

/// Outcome of the auto-reply safety rules for one mail (AI_MODES_DESIGN §5):
///   * any forced-demotion signal (attachment / amount / important contact)
///     → `demoted` (E4 non-disableable),
///   * else a bulk/marketing/CC mail → `skipped` (E2 filter),
///   * else → `sent_auto`.
pub fn evaluate(fx: &Fixture) -> &'static str {
    if fx.has_attachment || fx.has_amount || fx.important_contact {
        "demoted"
    } else if fx.is_bulk {
        "skipped"
    } else {
        "sent_auto"
    }
}
