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
//! Windows account on this machine. On other platforms (CI / unit tests)
//! the functions are identity transforms — Unduhin ships on Windows, and a
//! plaintext fallback keeps the cross-platform test build working.
//!
//! Values are stored self-describing: [`protect`] returns `dpapi:v1:<b64>`
//! when encryption succeeds and the raw input otherwise, and [`unprotect`]
//! decrypts only the tagged form, passing through legacy plaintext rows
//! unchanged. That makes the change backward-compatible with no migration.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

const TAG: &str = "dpapi:v1:";

/// Encrypt `plaintext` for storage. On success returns `dpapi:v1:<base64>`;
/// if encryption is unavailable (non-Windows) or fails for any reason,
/// returns the plaintext unchanged so a download is never lost merely
/// because DPAPI hiccuped (worst case degrades to the prior behavior).
pub(crate) fn protect(plaintext: &str) -> String {
    match dpapi_protect(plaintext.as_bytes()) {
        Some(cipher) => format!("{TAG}{}", STANDARD.encode(cipher)),
        None => plaintext.to_string(),
    }
}

/// Reverse of [`protect`]. A `dpapi:v1:`-tagged value is base64-decoded and
/// decrypted; any other value (legacy plaintext) is returned as-is. If a
/// tagged value cannot be decrypted (corruption, different user/machine)
/// the original stored string is returned so the caller's JSON parse fails
/// loudly rather than silently yielding wrong headers.
pub(crate) fn unprotect(stored: &str) -> String {
    let Some(b64) = stored.strip_prefix(TAG) else {
        return stored.to_string();
    };
    let Ok(cipher) = STANDARD.decode(b64) else {
        return stored.to_string();
    };
    match dpapi_unprotect(&cipher) {
        Some(plain) => String::from_utf8_lossy(&plain).into_owned(),
        None => stored.to_string(),
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

#[cfg(not(target_os = "windows"))]
fn dpapi_protect(_plaintext: &[u8]) -> Option<Vec<u8>> {
    None
}

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
}
