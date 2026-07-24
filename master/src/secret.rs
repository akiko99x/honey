//! Secret-at-rest encryption for VPN credentials (user passwords, REALITY
//! private keys). Values are encrypted with a process-wide master key (32 bytes,
//! from `HONEY_SECRET_KEY`, base64) using XChaCha20-Poly1305 and stored as
//! `enc:v1:<base64(nonce||ciphertext)>`.
//!
//! Backward compatible: values without the `enc:v1:` prefix are treated as
//! legacy plaintext, so an existing db can be upgraded by `reencrypt`. Runtime
//! commands refuse to start without a key; the internal passthrough exists only
//! so key-management and migration code can recognise legacy rows.
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 24; // XChaCha20 nonce

static CIPHER: OnceLock<Option<XChaCha20Poly1305>> = OnceLock::new();

/// Initialise the process cipher from an optional base64 key. Call once at start.
pub fn init(key_b64: Option<&str>) -> Result<()> {
    let cipher = match key_b64 {
        Some(raw) if !raw.trim().is_empty() => {
            let bytes = STANDARD
                .decode(raw.trim())
                .context("HONEY_SECRET_KEY must be base64")?;
            if bytes.len() != 32 {
                bail!(
                    "HONEY_SECRET_KEY must decode to 32 bytes, got {}",
                    bytes.len()
                );
            }
            Some(XChaCha20Poly1305::new(Key::from_slice(&bytes)))
        }
        _ => None,
    };
    CIPHER
        .set(cipher)
        .map_err(|_| anyhow!("secret cipher already initialised"))
}

pub fn is_enabled() -> bool {
    matches!(CIPHER.get(), Some(Some(_)))
}

fn cipher() -> Option<&'static XChaCha20Poly1305> {
    CIPHER.get().and_then(|c| c.as_ref())
}

/// Encrypt a value for storage. Runtime callers must enforce a configured key.
pub fn encrypt(plaintext: &str) -> Result<String> {
    let Some(cipher) = cipher() else {
        return Ok(plaintext.to_string());
    };
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|e| anyhow!("rng failed: {e}"))?;
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| anyhow!("encrypt failed"))?;
    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(format!("{PREFIX}{}", STANDARD.encode(blob)))
}

/// Decrypt a stored value. Legacy plaintext (no prefix) is returned as-is.
pub fn decrypt(stored: &str) -> Result<String> {
    let Some(b64) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let cipher =
        cipher().ok_or_else(|| anyhow!("encrypted value found but HONEY_SECRET_KEY is not set"))?;
    let blob = STANDARD.decode(b64).context("bad ciphertext base64")?;
    if blob.len() <= NONCE_LEN {
        bail!("ciphertext too short");
    }
    let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
    let plaintext = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("decrypt failed (wrong key?)"))?;
    String::from_utf8(plaintext).context("decrypted value is not utf-8")
}

fn cipher_from_b64(key_b64: &str) -> Result<XChaCha20Poly1305> {
    let bytes = STANDARD
        .decode(key_b64.trim())
        .context("key must be base64")?;
    if bytes.len() != 32 {
        bail!("key must decode to 32 bytes, got {}", bytes.len());
    }
    Ok(XChaCha20Poly1305::new(Key::from_slice(&bytes)))
}

/// Re-encrypt one stored value from `old_key_b64` to `new_key_b64` — the core of
/// key rotation. Legacy plaintext (no prefix) is encrypted under the new key.
pub fn rekey_value(old_key_b64: &str, new_key_b64: &str, stored: &str) -> Result<String> {
    let plaintext = match stored.strip_prefix(PREFIX) {
        Some(b64) => {
            let cipher = cipher_from_b64(old_key_b64)?;
            let blob = STANDARD.decode(b64).context("bad ciphertext base64")?;
            if blob.len() <= NONCE_LEN {
                bail!("ciphertext too short");
            }
            let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
            let pt = cipher
                .decrypt(XNonce::from_slice(nonce), ciphertext)
                .map_err(|_| anyhow!("decrypt failed (wrong old key?)"))?;
            String::from_utf8(pt).context("decrypted value is not utf-8")?
        }
        None => stored.to_string(),
    };
    let cipher = cipher_from_b64(new_key_b64)?;
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|e| anyhow!("rng failed: {e}"))?;
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| anyhow!("encrypt failed"))?;
    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(format!("{PREFIX}{}", STANDARD.encode(blob)))
}

/// Generate a fresh base64 master key for the operator to store as a secret.
pub fn generate_key_b64() -> Result<String> {
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| anyhow!("rng failed: {e}"))?;
    Ok(STANDARD.encode(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_legacy_passthrough() {
        init(Some(&generate_key_b64().unwrap())).unwrap();
        let enc = encrypt("s3cret").unwrap();
        assert!(enc.starts_with(PREFIX));
        assert_eq!(decrypt(&enc).unwrap(), "s3cret");
        // legacy plaintext survives untouched
        assert_eq!(decrypt("plain").unwrap(), "plain");
    }
}
