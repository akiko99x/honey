//! Human authentication primitives. Passwords use Argon2id PHC strings;
//! session and enrollment credentials are random bearer tokens whose SHA-256
//! digests, never the original tokens, are persisted.
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct Identity {
    pub admin_id: Option<Uuid>,
    pub username: String,
    pub role: String,
    #[serde(skip)]
    pub session_hash: Option<Vec<u8>>,
    // custom RBAC matrix (domain -> level); when Some it is authoritative.
    #[serde(skip)]
    pub permissions: Option<std::collections::HashMap<String, i64>>,
}

impl Identity {
    pub fn legacy() -> Self {
        Self {
            admin_id: None,
            username: "legacy-api-token".into(),
            role: "owner".into(),
            session_hash: None,
            permissions: None,
        }
    }

    pub fn permits(&self, required: &str) -> bool {
        role_rank(&self.role) >= role_rank(required)
    }

    /// Custom-RBAC check for a `need` level on a domain; `dashboard` (read-only
    /// personal/overview surfaces) is always permitted for a valid admin.
    pub fn permits_domain(&self, domain: &str, need: i64) -> bool {
        if domain == "dashboard" {
            return true;
        }
        self.permissions
            .as_ref()
            .and_then(|p| p.get(domain).copied())
            .unwrap_or(0)
            >= need
    }
}

pub fn valid_role(role: &str) -> bool {
    matches!(role, "owner" | "admin" | "operator" | "viewer" | "reseller")
}

impl Identity {
    pub fn is_reseller(&self) -> bool {
        self.role == "reseller"
    }
}

fn role_rank(role: &str) -> u8 {
    // reseller sits outside the linear rank ladder: it can write users but only
    // its own, so it goes through a dedicated scope allowlist, never rank checks.
    match role {
        "owner" => 3,
        "admin" => 2,
        "operator" => 1,
        _ => 0,
    }
}

pub fn hash_password(password: &str) -> Result<String> {
    if password.len() < 10 {
        anyhow::bail!("password must contain at least 10 characters");
    }
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt).map_err(|error| anyhow!("rng failed: {error}"))?;
    let salt = SaltString::encode_b64(&salt)
        .map_err(|error| anyhow!("could not encode password salt: {error}"))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow!("password hashing failed: {error}"))?
        .to_string())
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    let Ok(hash) = PasswordHash::new(encoded) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .is_ok()
}

/// Burn one verification against a throwaway hash. A login for a username that
/// does not exist (or is disabled) must cost the same as a real one — argon2 is
/// slow enough that skipping it would let an attacker enumerate admin names by
/// response latency alone. The hash is built once, from a random password, so
/// it carries exactly the parameters `verify_password` would face for real.
pub fn verify_dummy(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let encoded = DUMMY.get_or_init(|| {
        random_token()
            .and_then(|secret| hash_password(&secret))
            .unwrap_or_default()
    });
    if !encoded.is_empty() {
        let _ = verify_password(password, encoded);
    }
}

pub fn random_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| anyhow!("rng failed: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Tamper-evident hash for an audit entry, chaining it to the previous one.
/// Recomputable from the stored fields to detect deletion/modification.
pub fn audit_chain_hash(
    prev: Option<&[u8]>,
    id: i64,
    actor: Option<&str>,
    action: &str,
    resource_type: &str,
    resource_id: Option<&str>,
    details: &str,
    created_micros: i64,
) -> Vec<u8> {
    let mut h = Sha256::new();
    if let Some(p) = prev {
        h.update(p);
    }
    h.update(id.to_be_bytes());
    for field in [
        actor.unwrap_or(""),
        action,
        resource_type,
        resource_id.unwrap_or(""),
        details,
    ] {
        h.update(field.as_bytes());
        h.update([0u8]);
    }
    h.update(created_micros.to_be_bytes());
    h.finalize().to_vec()
}

pub fn generate_recovery_code() -> Result<String> {
    let mut bytes = [0u8; 10];
    getrandom::getrandom(&mut bytes).map_err(|error| anyhow!("rng failed: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{:02X}", byte)).collect())
}

pub fn normalize_recovery_code(value: &str) -> Option<String> {
    let normalized: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if normalized.len() != 20 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(normalized)
}

pub fn spec_hash(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}

// --- TOTP (RFC 6238) two-factor -------------------------------------------

use totp_rs::{Algorithm, Secret, TOTP};

/// A fresh base32 TOTP secret for a Google-Authenticator-style app.
pub fn generate_totp_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

fn totp_for(secret_b32: &str, account: &str) -> Result<TOTP> {
    let bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| anyhow!("invalid totp secret: {e:?}"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("honey".to_string()),
        account.to_string(),
    )
    .map_err(|e| anyhow!("totp: {e}"))
}

/// `otpauth://` URL to render as a QR for the authenticator app.
pub fn totp_provisioning_url(secret_b32: &str, account: &str) -> Result<String> {
    Ok(totp_for(secret_b32, account)?.get_url())
}

/// Verify a 6-digit code against the secret (±1 step of skew).
pub fn verify_totp(secret_b32: &str, code: &str) -> bool {
    totp_for(secret_b32, "honey")
        .and_then(|totp| totp.check_current(code).map_err(|e| anyhow!("{e}")))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // If the throwaway hash ever failed to build, verify_dummy would quietly
    // become a no-op and the login timing equalization would stop working
    // without anything failing loudly — so assert the construction path holds.
    #[test]
    fn dummy_verification_has_a_real_hash_to_check_against() {
        let encoded = hash_password(&random_token().expect("rng")).expect("dummy hash builds");
        assert!(PasswordHash::new(&encoded).is_ok());
        assert!(!verify_password("not the dummy password", &encoded));
        verify_dummy("not the dummy password");
    }

    #[test]
    fn password_and_tokens_are_one_way() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
        let token = random_token().unwrap();
        assert_ne!(token.as_bytes(), token_hash(&token));
    }

    #[test]
    fn recovery_codes_are_normalized_without_becoming_weak() {
        let code = generate_recovery_code().unwrap();
        assert_eq!(code.len(), 20);
        assert_eq!(normalize_recovery_code(&code), Some(code.clone()));
        assert_eq!(
            normalize_recovery_code(&format!("{}-{}", &code[..10], &code[10..])),
            Some(code)
        );
        assert!(normalize_recovery_code("not-a-code").is_none());
    }
}
