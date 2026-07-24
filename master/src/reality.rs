//! REALITY x25519 keypair + short-id generation for one-click inbound setup.
//!
//! Output matches what sing-box / xray expect and what the API validator accepts:
//! keys are unpadded base64url of the 32-byte x25519 keys (43 chars), and the
//! short id is an even-length hex string. The public key is derived from the
//! private one so the client subscription can carry it.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use x25519_dalek::{PublicKey, StaticSecret};

pub struct RealityKeypair {
    pub private_key: String,
    pub public_key: String,
    pub short_id: String,
}

pub fn generate() -> Result<RealityKeypair> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow!("rng failed: {e}"))?;
    let secret = StaticSecret::from(seed);
    let public = PublicKey::from(&secret);

    let mut sid = [0u8; 4];
    getrandom::getrandom(&mut sid).map_err(|e| anyhow!("rng failed: {e}"))?;

    Ok(RealityKeypair {
        private_key: URL_SAFE_NO_PAD.encode(secret.to_bytes()),
        public_key: URL_SAFE_NO_PAD.encode(public.to_bytes()),
        short_id: sid.iter().map(|b| format!("{b:02x}")).collect(),
    })
}
