//! At-rest protection for sensitive captured data.
//!
//! Browser captures can include `Cookie` (HttpOnly included) and
//! `Authorization` headers, which we persist so a download can be resumed
//! or retried against cookie-/auth-gated CDNs. Storing them as plaintext in
//! the SQLite file is a downgrade from the browser's encrypted-at-rest
//! store: a backup, a synced profile, or another local process could read
//! them.
//!
//! On Windows we wrap DPAPI (`CryptProtectData` / `CryptUnprotectData`)
//! scoped to the current user, so the ciphertext is readable only by this
//! Windows account on this machine.
//!
//! On macOS there is no DPAPI equivalent, so we hold one AES-256-GCM key in
//! the login Keychain and wrap values with it. The key is stored rather
//! than the values themselves: `protect` is a synchronous pure function
//! with no row identity, so a per-row Keychain item would need a download
//! id threaded through every call site, would strand orphaned items when
//! rows are deleted, and would multiply access prompts by the number of
//! downloads. One key, read at most once per process, bounds all of that.
//!
//! Known caveat, and it follows directly from shipping unsigned: macOS ties
//! a Keychain item's ACL to the creating binary's code signature. An
//! unsigned app has no stable identity, so the system may re-prompt for
//! access after an update. There is no fix without a Developer ID —
//! ad-hoc signing produces a fresh identity on every build, so it does not
//! stabilize anything.
//!
//! On every other platform the functions are identity transforms, which
//! keeps the cross-platform test build working.
//!
//! Values are stored self-describing: [`protect`] returns
//! `<scheme>:v1:<b64>` when encryption succeeds and the raw input
//! otherwise, and [`unprotect`] decrypts only a tagged form, passing
//! through legacy plaintext rows unchanged. Both schemes stay recognized on
//! both platforms, so a database copied between machines degrades to an
//! unreadable-but-intact row rather than silently yielding wrong headers.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

const TAG: &str = "dpapi:v1:";
const KEYCHAIN_TAG: &str = "keychain:v1:";

/// Encrypt `plaintext` for storage.
///
/// On success returns `<scheme>:v1:<base64>`; if encryption is unavailable
/// or fails for any reason, returns the plaintext unchanged so a download
/// is never lost merely because DPAPI hiccuped or the Keychain was locked.
/// The worst case degrades to the prior behavior rather than to an error.
pub(crate) fn protect(plaintext: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        if let Some(cipher) = dpapi_protect(plaintext.as_bytes()) {
            return format!("{TAG}{}", STANDARD.encode(cipher));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(cipher) = keychain::protect(plaintext.as_bytes()) {
            return format!("{KEYCHAIN_TAG}{}", STANDARD.encode(cipher));
        }
    }
    plaintext.to_string()
}

/// Reverse of [`protect`]. A tagged value is base64-decoded and decrypted;
/// any other value (legacy plaintext) is returned as-is. If a tagged value
/// cannot be decrypted (corruption, a different user, machine, or platform)
/// the original stored string is returned so the caller's JSON parse fails
/// loudly rather than silently yielding wrong headers.
pub(crate) fn unprotect(stored: &str) -> String {
    // Both schemes are matched on every platform. A row written on Windows
    // and read on macOS has no key to decrypt with, and returning the
    // stored form is exactly the loud failure we want.
    if let Some(b64) = stored.strip_prefix(TAG) {
        let Ok(cipher) = STANDARD.decode(b64) else {
            return stored.to_string();
        };
        return match dpapi_unprotect(&cipher) {
            Some(plain) => String::from_utf8_lossy(&plain).into_owned(),
            None => stored.to_string(),
        };
    }
    if let Some(b64) = stored.strip_prefix(KEYCHAIN_TAG) {
        let Ok(cipher) = STANDARD.decode(b64) else {
            return stored.to_string();
        };
        return match keychain_unprotect(&cipher) {
            Some(plain) => String::from_utf8_lossy(&plain).into_owned(),
            None => stored.to_string(),
        };
    }
    stored.to_string()
}

#[cfg(target_os = "macos")]
fn keychain_unprotect(ciphertext: &[u8]) -> Option<Vec<u8>> {
    keychain::unprotect(ciphertext)
}

#[cfg(not(target_os = "macos"))]
fn keychain_unprotect(_ciphertext: &[u8]) -> Option<Vec<u8>> {
    None
}

/// Keychain-backed AES-256-GCM wrapping.
///
/// Layout of the sealed blob is `nonce || ciphertext || tag`, with the
/// 96-bit nonce generated fresh per call.
#[cfg(target_os = "macos")]
mod keychain {
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
    use ring::rand::{SecureRandom, SystemRandom};
    use std::sync::OnceLock;

    /// Matches the app's bundle identifier so the item is recognizable in
    /// Keychain Access.
    const SERVICE: &str = "com.unduhin.app";
    const ACCOUNT: &str = "header-encryption-key";
    const KEY_LEN: usize = 32;

    /// Read the Keychain at most once per process. That is what bounds how
    /// often an unsigned build can prompt the user: once, not once per
    /// captured header.
    static MASTER_KEY: OnceLock<Option<[u8; KEY_LEN]>> = OnceLock::new();

    fn master_key() -> Option<&'static [u8; KEY_LEN]> {
        MASTER_KEY.get_or_init(load_or_create).as_ref()
    }

    fn load_or_create() -> Option<[u8; KEY_LEN]> {
        use security_framework::passwords::{get_generic_password, set_generic_password};

        if let Ok(existing) = get_generic_password(SERVICE, ACCOUNT) {
            match <[u8; KEY_LEN]>::try_from(existing.as_slice()) {
                Ok(key) => return Some(key),
                Err(_) => {
                    // Someone else wrote this item, or it predates a format
                    // change. Overwriting is safe: anything sealed with the
                    // old key already fails closed in `unprotect`.
                    tracing::warn!(
                        len = existing.len(),
                        "keychain item has an unexpected length; regenerating"
                    );
                }
            }
        }

        let mut key = [0u8; KEY_LEN];
        if SystemRandom::new().fill(&mut key).is_err() {
            tracing::warn!("failed to generate a header-encryption key");
            return None;
        }
        match set_generic_password(SERVICE, ACCOUNT, &key) {
            Ok(()) => Some(key),
            Err(e) => {
                // Locked keychain, denied access, or no GUI session (CI).
                // Callers fall back to plaintext, which is what this
                // platform did before.
                tracing::warn!(error = %e, "keychain unavailable; headers will be stored unencrypted");
                None
            }
        }
    }

    fn key_handle() -> Option<LessSafeKey> {
        UnboundKey::new(&AES_256_GCM, master_key()?)
            .ok()
            .map(LessSafeKey::new)
    }

    pub(super) fn protect(plaintext: &[u8]) -> Option<Vec<u8>> {
        let key = key_handle()?;
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new().fill(&mut nonce).ok()?;

        let mut sealed = plaintext.to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut sealed,
        )
        .ok()?;

        let mut out = Vec::with_capacity(NONCE_LEN + sealed.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Some(out)
    }

    pub(super) fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
        if blob.len() <= NONCE_LEN {
            return None;
        }
        let key = key_handle()?;
        let (nonce, rest) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;

        let mut buf = rest.to_vec();
        let plain = key.open_in_place(nonce, Aad::empty(), &mut buf).ok()?;
        Some(plain.to_vec())
    }
}

#[cfg(target_os = "windows")]
fn dpapi_protect(plaintext: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `plaintext` for the duration of the call;
    // DPAPI copies it and never mutates the input. `output` is owned by the
    // API until we copy it out and `LocalFree` it below.
    unsafe {
        CryptProtectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .ok()?;
    }
    Some(take_and_free_blob(&output))
}

#[cfg(target_os = "windows")]
fn dpapi_unprotect(ciphertext: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: see `dpapi_protect`.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .ok()?;
    }
    Some(take_and_free_blob(&output))
}

/// Copy a DPAPI-allocated output blob into an owned `Vec` and release the
/// buffer with `LocalFree` (DPAPI allocates `pbData` with `LocalAlloc`).
#[cfg(target_os = "windows")]
fn take_and_free_blob(
    blob: &windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB,
) -> Vec<u8> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};

    // SAFETY: `pbData`/`cbData` were populated by a successful DPAPI call.
    let out = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(blob.pbData as *mut core::ffi::c_void)));
    }
    out
}

// No `dpapi_protect` counterpart here: `protect` only reaches it inside a
// `cfg(windows)` block, so a stub would be dead code. `dpapi_unprotect` is
// different — `unprotect` matches the `dpapi:v1:` tag on every platform so
// a database copied from a Windows machine fails closed instead of being
// mistaken for plaintext.
#[cfg(not(target_os = "windows"))]
fn dpapi_unprotect(_ciphertext: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_value() {
        let secret = r#"[["Cookie","sid=abc; auth=xyz"],["Referer","https://x/"]]"#;
        let stored = protect(secret);
        assert_eq!(unprotect(&stored), secret);
    }

    #[test]
    fn legacy_plaintext_reads_through() {
        // Rows written before this change have no tag; they must read back
        // verbatim.
        let legacy = r#"[["Referer","https://x/"]]"#;
        assert_eq!(unprotect(legacy), legacy);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_output_is_tagged_and_not_plaintext() {
        let secret = "sid=supersecret";
        let stored = protect(secret);
        assert!(stored.starts_with(TAG), "stored value should be tagged");
        assert!(
            !stored.contains("supersecret"),
            "ciphertext must not contain the plaintext"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_either_encrypts_or_degrades_visibly() {
        // A headless CI runner has no unlocked login keychain, so this
        // legitimately takes the plaintext fallback there. Assert the
        // invariant that holds either way: the value is either sealed and
        // tagged, or untouched — never a tagged blob we cannot read back.
        let secret = "sid=supersecret";
        let stored = protect(secret);

        if stored.starts_with(KEYCHAIN_TAG) {
            assert!(
                !stored.contains("supersecret"),
                "ciphertext must not contain the plaintext"
            );
            assert_eq!(unprotect(&stored), secret, "sealed value must round-trip");
        } else {
            assert_eq!(
                stored, secret,
                "without a keychain the value must pass through unchanged"
            );
        }
    }

    #[test]
    fn foreign_scheme_fails_closed_rather_than_reading_as_plaintext() {
        // A row written by the other platform must not be mistaken for a
        // usable value. `unprotect` hands back the stored form so the
        // caller's JSON parse fails loudly.
        let windows_row = format!("{TAG}{}", STANDARD.encode(b"not really dpapi output"));
        let keychain_row = format!("{KEYCHAIN_TAG}{}", STANDARD.encode(b"not really sealed"));
        for row in [windows_row, keychain_row] {
            let out = unprotect(&row);
            assert!(
                out == row || !out.contains("not really"),
                "a foreign row must never decode to its raw inner bytes: {out:?}"
            );
        }
    }
}
