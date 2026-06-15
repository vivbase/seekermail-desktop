//! OS Keychain credential vault (T006, A2 minimal).
//!
//! Credentials live **only** in the OS Keychain — never the DB, the frontend,
//! env files, or logs (08 §7, 01 `accounts` note, 09 §5). This module exposes a
//! tiny `set / get / delete` surface keyed by `{account_id}:{kind}` under the
//! service name `"SeekerMail"`.
//!
//! macOS is the only backend implemented this card (`security-framework`). Other
//! targets compile to a stub that denies access, so CI/dev on Linux still builds;
//! Windows Credential Manager arrives with cross-platform support at v1.0.

use std::fmt;

use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::AppResult;

/// Keychain service name — one item namespace for the whole app.
const SERVICE: &str = "SeekerMail";

/// Which credential an item holds. Combined with the account id to form the
/// Keychain item account field, so all of an account's secrets can be cleared
/// together on account deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredKind {
    ImapPassword,
    SmtpPassword,
    OAuthAccessToken,
    OAuthRefreshToken,
    /// Unix-seconds expiry of the access token, stored as a plain number string so
    /// the scheduler can check freshness without decrypting the token (T018 §3).
    OAuthExpiry,
    AiApiKey,
}

impl CredKind {
    /// Stable token used in the Keychain item key. Never localized, never changed.
    pub fn as_str(self) -> &'static str {
        match self {
            CredKind::ImapPassword => "imap_password",
            CredKind::SmtpPassword => "smtp_password",
            CredKind::OAuthAccessToken => "oauth_access_token",
            CredKind::OAuthRefreshToken => "oauth_refresh_token",
            CredKind::OAuthExpiry => "oauth_expiry",
            CredKind::AiApiKey => "ai_api_key",
        }
    }

    /// All kinds — used by [`Keychain::delete_all`] to purge an account.
    const ALL: [CredKind; 6] = [
        CredKind::ImapPassword,
        CredKind::SmtpPassword,
        CredKind::OAuthAccessToken,
        CredKind::OAuthRefreshToken,
        CredKind::OAuthExpiry,
        CredKind::AiApiKey,
    ];
}

/// A secret value. Zeroized on drop and redacted in `Debug`/logs so plaintext
/// never reaches a log line or a panic message (09 §5).
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    /// Borrow the plaintext. Callers must not log or persist the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Secret(value)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Build the Keychain item account field: `{account_id}:{kind}`.
fn item_key(account_id: &Uuid, kind: CredKind) -> String {
    format!("{}:{}", account_id, kind.as_str())
}

/// Credential vault handle. Stateless — methods talk to the OS Keychain directly.
#[derive(Debug, Clone, Default)]
pub struct Keychain;

impl Keychain {
    pub fn new() -> Self {
        Keychain
    }

    /// Store (or overwrite) a secret for `{account_id}:{kind}`.
    pub fn set(&self, account_id: &Uuid, kind: CredKind, secret: &Secret) -> AppResult<()> {
        let key = item_key(account_id, kind);
        #[cfg(debug_assertions)]
        if let Some(path) = dev_vault::active() {
            return dev_vault::set(&path, &key, secret.expose());
        }
        backend::set(&key, secret.expose())
    }

    /// Fetch a secret. Returns `Ok(None)` when the item does not exist (not an
    /// error); `AUTH_KEYCHAIN_DENIED` when access is refused.
    pub fn get(&self, account_id: &Uuid, kind: CredKind) -> AppResult<Option<Secret>> {
        let key = item_key(account_id, kind);
        #[cfg(debug_assertions)]
        if let Some(path) = dev_vault::active() {
            return dev_vault::get(&path, &key);
        }
        backend::get(&key)
    }

    /// Delete a secret. Deleting a missing item is a no-op success.
    pub fn delete(&self, account_id: &Uuid, kind: CredKind) -> AppResult<()> {
        let key = item_key(account_id, kind);
        #[cfg(debug_assertions)]
        if let Some(path) = dev_vault::active() {
            return dev_vault::delete(&path, &key);
        }
        backend::delete(&key)
    }

    /// Remove every credential for an account (called on account/data deletion).
    pub fn delete_all(&self, account_id: &Uuid) -> AppResult<()> {
        for kind in CredKind::ALL {
            self.delete(account_id, kind)?;
        }
        Ok(())
    }

    // ── OAuth token helpers (T015/T018) ─────────────────────────────────────

    /// Store a full OAuth token set (access + optional refresh + expiry). The
    /// caller zeroizes its own plaintext copies after this returns.
    pub fn store_oauth(
        &self,
        account_id: &Uuid,
        access_token: &str,
        refresh_token: Option<&str>,
        expiry_unix: i64,
    ) -> AppResult<()> {
        self.set(
            account_id,
            CredKind::OAuthAccessToken,
            &Secret::new(access_token),
        )?;
        if let Some(rt) = refresh_token {
            // Microsoft rotates the refresh token; only overwrite when provided.
            self.set(account_id, CredKind::OAuthRefreshToken, &Secret::new(rt))?;
        }
        self.store_oauth_expiry(account_id, expiry_unix)
    }

    /// Read the refresh token, if any.
    pub fn get_refresh_token(&self, account_id: &Uuid) -> AppResult<Option<Secret>> {
        self.get(account_id, CredKind::OAuthRefreshToken)
    }

    /// Store the access-token expiry as a number string.
    pub fn store_oauth_expiry(&self, account_id: &Uuid, expiry_unix: i64) -> AppResult<()> {
        self.set(
            account_id,
            CredKind::OAuthExpiry,
            &Secret::new(expiry_unix.to_string()),
        )
    }

    /// Read the access-token expiry (Unix seconds), if present.
    pub fn get_oauth_expiry(&self, account_id: &Uuid) -> AppResult<Option<i64>> {
        Ok(self
            .get(account_id, CredKind::OAuthExpiry)?
            .and_then(|s| s.expose().parse::<i64>().ok()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Debug-only developer vault (NOT a production backend).
//
// When `SEEKERMAIL_DEV_VAULT` names a file, **debug** builds route every
// credential read/write to that JSON file instead of the OS Keychain. This exists
// for exactly one reason: a developer iterating on the UI must not be interrupted
// by an OS Keychain authorization prompt on every rebuild (the dev binary's code
// signature is not stable, so macOS re-prompts on each launch).
//
// Hard guarantees:
//   * Gated on `#[cfg(debug_assertions)]` — `tauri build` (release) compiles this
//     module out entirely, so shipped binaries can ONLY use the OS Keychain.
//   * Inert unless `SEEKERMAIL_DEV_VAULT` is set — a debug build with the var
//     unset behaves exactly as before (OS Keychain).
//   * Values are base64-wrapped under `{account_id}:{kind}` keys, mirroring the
//     Keychain item namespace, so `delete_all` and OAuth helpers work unchanged.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(debug_assertions)]
mod dev_vault {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use base64::{engine::general_purpose::STANDARD, Engine as _};

    use super::Secret;
    use crate::error::{AppError, AppResult};

    // Serializes concurrent read-modify-write on the vault file (the poll task and
    // command handlers can touch it simultaneously).
    static GUARD: Mutex<()> = Mutex::new(());

    /// The vault file path, if dev-vault mode is active for this process.
    pub fn active() -> Option<PathBuf> {
        match std::env::var_os("SEEKERMAIL_DEV_VAULT") {
            Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
            _ => None,
        }
    }

    fn io(e: impl std::fmt::Display) -> AppError {
        AppError::Internal(anyhow::anyhow!("dev vault: {e}"))
    }

    fn load(path: &Path) -> AppResult<BTreeMap<String, String>> {
        match std::fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => serde_json::from_slice(&bytes).map_err(io),
            Ok(_) => Ok(BTreeMap::new()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(io(e)),
        }
    }

    fn store(path: &Path, map: &BTreeMap<String, String>) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
        }
        let json = serde_json::to_vec_pretty(map).map_err(io)?;
        std::fs::write(path, json).map_err(io)
    }

    pub fn get(path: &Path, key: &str) -> AppResult<Option<Secret>> {
        let _g = GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let map = load(path)?;
        match map.get(key) {
            Some(enc) => {
                let raw = STANDARD.decode(enc).map_err(io)?;
                let value = String::from_utf8(raw).map_err(io)?;
                Ok(Some(Secret::new(value)))
            }
            None => Ok(None),
        }
    }

    pub fn set(path: &Path, key: &str, plaintext: &str) -> AppResult<()> {
        let _g = GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let mut map = load(path)?;
        map.insert(key.to_string(), STANDARD.encode(plaintext.as_bytes()));
        store(path, &map)
    }

    pub fn delete(path: &Path, key: &str) -> AppResult<()> {
        let _g = GUARD.lock().unwrap_or_else(|p| p.into_inner());
        let mut map = load(path)?;
        map.remove(key);
        store(path, &map)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// macOS backend — security-framework Generic Password items.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
mod backend {
    use super::{Secret, SERVICE};
    use crate::error::{AppError, AppResult};

    // errSecItemNotFound — the only "error" we treat as a plain miss.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub fn set(item_key: &str, plaintext: &str) -> AppResult<()> {
        security_framework::passwords::set_generic_password(SERVICE, item_key, plaintext.as_bytes())
            .map_err(|_| AppError::AuthKeychainDenied)
    }

    pub fn get(item_key: &str) -> AppResult<Option<Secret>> {
        match security_framework::passwords::get_generic_password(SERVICE, item_key) {
            Ok(bytes) => {
                let value = String::from_utf8(bytes)
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("non-utf8 secret: {e}")))?;
                Ok(Some(Secret::new(value)))
            }
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(_) => Err(AppError::AuthKeychainDenied),
        }
    }

    pub fn delete(item_key: &str) -> AppResult<()> {
        match security_framework::passwords::delete_generic_password(SERVICE, item_key) {
            Ok(()) => Ok(()),
            // Deleting a missing item is success from the caller's point of view.
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(_) => Err(AppError::AuthKeychainDenied),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Windows backend — Credential Manager via the `keyring` crate (T114). Mirrors
// the macOS `set / get / delete` surface; `{account_id}:{kind}` items live under
// the `SeekerMail` service. `delete_all` (above) enumerates every `CredKind` and
// calls `delete`, which treats a missing item as success — identical semantics to
// macOS, so the platform-agnostic `Keychain` API is unchanged.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(target_os = "windows")]
mod backend {
    use keyring::{Entry, Error as KeyringError};

    use super::{Secret, SERVICE};
    use crate::error::{AppError, AppResult};

    /// Map a keyring failure to the crate error. A missing item is handled by the
    /// callers (not an error); everything else is a denied/locked credential store.
    fn denied(_e: KeyringError) -> AppError {
        AppError::AuthKeychainDenied
    }

    fn entry(item_key: &str) -> AppResult<Entry> {
        Entry::new(SERVICE, item_key).map_err(denied)
    }

    pub fn set(item_key: &str, plaintext: &str) -> AppResult<()> {
        entry(item_key)?.set_password(plaintext).map_err(denied)
    }

    pub fn get(item_key: &str) -> AppResult<Option<Secret>> {
        match entry(item_key)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            // A missing credential is a plain miss, not an error.
            Err(KeyringError::NoEntry) => Ok(None),
            Err(e) => Err(denied(e)),
        }
    }

    pub fn delete(item_key: &str) -> AppResult<()> {
        match entry(item_key)?.delete_credential() {
            Ok(()) => Ok(()),
            // Deleting a missing item is success (matches macOS).
            Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(denied(e)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Other targets (Linux CI/dev) — stub that denies writes but builds cleanly.
// v0.1 never stores creds on these targets, so denial is harmless.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod backend {
    use super::Secret;
    use crate::error::{AppError, AppResult};

    pub fn set(_item_key: &str, _plaintext: &str) -> AppResult<()> {
        Err(AppError::AuthKeychainDenied)
    }

    pub fn get(_item_key: &str) -> AppResult<Option<Secret>> {
        Ok(None)
    }

    pub fn delete(_item_key: &str) -> AppResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let s = Secret::new("hunter2-super-secret");
        let shown = format!("{s:?}");
        assert_eq!(shown, "Secret(***)");
        assert!(!shown.contains("hunter2"));
    }

    #[test]
    fn item_key_is_account_then_kind() {
        let id = Uuid::nil();
        assert_eq!(
            item_key(&id, CredKind::ImapPassword),
            "00000000-0000-0000-0000-000000000000:imap_password"
        );
    }

    // Full set/get/delete roundtrip needs a real (or temporary) Keychain, which
    // CI macOS runners can provide. Ignored by default; run locally on macOS with
    // `cargo test --ignored`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires interactive macOS Keychain access"]
    fn set_get_delete_roundtrip() {
        let kc = Keychain::new();
        let id = Uuid::new_v4();
        let secret = Secret::new("roundtrip-value");
        kc.set(&id, CredKind::ImapPassword, &secret).unwrap();
        let got = kc.get(&id, CredKind::ImapPassword).unwrap().unwrap();
        assert_eq!(got.expose(), "roundtrip-value");
        kc.delete(&id, CredKind::ImapPassword).unwrap();
        assert!(kc.get(&id, CredKind::ImapPassword).unwrap().is_none());
    }
}
