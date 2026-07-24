//! Resolve the at-rest master key from one of several backends so it need not
//! live in a plain environment variable. Precedence — first configured source
//! wins; a configured-but-failing source is a hard error; no configuration at
//! all yields `None` (dev plaintext mode, unchanged):
//!
//!   1. `HONEY_SECRET_KEY`          direct base64 value (default, backward-compat)
//!   2. `HONEY_SECRET_KEY_FILE`     path to a file holding the base64 key
//!                                  (Docker/K8s secrets, systemd credentials)
//!   3. `HONEY_VAULT_ADDR`          HashiCorp Vault KV read (see below)
//!   4. `HONEY_SECRET_KEY_COMMAND`  shell command whose stdout is the key
//!                                  (universal hatch: AWS/GCP secret managers, `pass`, …)
//!
//! Vault KV v2 (default): `GET {addr}/v1/{mount}/data/{path}` with `X-Vault-Token`,
//! reading field `HONEY_VAULT_FIELD` (default `key`) from `data.data`. Env knobs:
//! `HONEY_VAULT_TOKEN` (required), `HONEY_VAULT_PATH` (required, logical path under
//! the mount), `HONEY_VAULT_MOUNT` (default `secret`), `HONEY_VAULT_KV_VERSION`
//! (`1` selects the legacy KV v1 layout `data.<field>`).
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

static BACKEND: OnceLock<&'static str> = OnceLock::new();

/// The backend that supplied the active key, for read-only status surfaces.
/// `"none"` until [`resolve`] runs or when no source is configured.
pub fn active_backend() -> &'static str {
    BACKEND.get().copied().unwrap_or("none")
}

pub struct Resolved {
    pub key: Option<String>,
    pub backend: &'static str,
}

/// Read an env var, treating empty/whitespace as unset.
fn env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

fn require_env(name: &str) -> Result<String> {
    env(name).ok_or_else(|| anyhow!("{name} is required for this secret backend"))
}

/// Resolve the master key from the highest-precedence configured backend.
pub async fn resolve() -> Result<Resolved> {
    let (key, backend) = if let Some(v) = env("HONEY_SECRET_KEY") {
        (Some(v), "env")
    } else if let Some(path) = env("HONEY_SECRET_KEY_FILE") {
        let raw = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("reading HONEY_SECRET_KEY_FILE at {path}"))?;
        (Some(raw.trim().to_string()), "file")
    } else if env("HONEY_VAULT_ADDR").is_some() {
        (Some(fetch_vault().await?), "vault")
    } else if let Some(cmd) = env("HONEY_SECRET_KEY_COMMAND") {
        (Some(run_command(&cmd).await?), "command")
    } else {
        (None, "none")
    };
    if key.as_deref().is_some_and(|k| k.is_empty()) {
        bail!("secret backend '{backend}' produced an empty key");
    }
    let _ = BACKEND.set(backend);
    Ok(Resolved { key, backend })
}

async fn fetch_vault() -> Result<String> {
    let addr = require_env("HONEY_VAULT_ADDR")?;
    let addr = addr.trim_end_matches('/');
    let token = require_env("HONEY_VAULT_TOKEN")?;
    let path = require_env("HONEY_VAULT_PATH")?;
    let mount = env("HONEY_VAULT_MOUNT").unwrap_or_else(|| "secret".to_string());
    let field = env("HONEY_VAULT_FIELD").unwrap_or_else(|| "key".to_string());
    let kv1 = env("HONEY_VAULT_KV_VERSION").as_deref() == Some("1");
    let path = path.trim_matches('/');
    let url = if kv1 {
        format!("{addr}/v1/{mount}/{path}")
    } else {
        format!("{addr}/v1/{mount}/data/{path}")
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let resp = client
        .get(&url)
        .header("X-Vault-Token", token)
        .send()
        .await
        .with_context(|| format!("contacting Vault at {addr}"))?
        .error_for_status()
        .context("Vault returned an error status")?;
    let json: serde_json::Value = resp.json().await.context("parsing Vault response")?;
    let data = if kv1 {
        &json["data"]
    } else {
        &json["data"]["data"]
    };
    let value = data
        .get(&field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("field '{field}' not found in Vault secret"))?;
    Ok(value.trim().to_string())
}

async fn run_command(cmd: &str) -> Result<String> {
    let output = if cfg!(windows) {
        tokio::process::Command::new("cmd")
            .args(["/C", cmd])
            .output()
            .await
    } else {
        tokio::process::Command::new("sh")
            .args(["-c", cmd])
            .output()
            .await
    }
    .with_context(|| "spawning HONEY_SECRET_KEY_COMMAND")?;
    if !output.status.success() {
        bail!("HONEY_SECRET_KEY_COMMAND exited with {}", output.status);
    }
    let value = String::from_utf8(output.stdout)
        .context("HONEY_SECRET_KEY_COMMAND output was not utf-8")?
        .trim()
        .to_string();
    if value.is_empty() {
        bail!("HONEY_SECRET_KEY_COMMAND produced no output");
    }
    Ok(value)
}
