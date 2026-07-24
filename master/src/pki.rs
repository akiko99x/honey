//! Short-lived node certificate issuance for one-time enrollment. The agent
//! generates its private key locally and submits only a CSR; master signs it
//! with the existing honey CA through OpenSSL.
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use tokio::process::Command;
use uuid::Uuid;

pub struct IssuedCertificate {
    pub certificate_pem: String,
    pub ca_pem: String,
    pub serial_number: String,
    pub fingerprint_sha256: String,
    pub subject: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
}

pub async fn issue_node_certificate(
    certs_dir: &Path,
    node_id: Uuid,
    csr_pem: &str,
) -> Result<IssuedCertificate> {
    if csr_pem.len() > 32 * 1024 || !csr_pem.contains("BEGIN CERTIFICATE REQUEST") {
        anyhow::bail!("invalid or oversized certificate request");
    }
    let ca_cert = certs_dir.join("ca.crt");
    let ca_key = certs_dir.join("ca.key");
    if !ca_cert.is_file() || !ca_key.is_file() {
        anyhow::bail!("CA files ca.crt and ca.key are required in HONEY_CERTS_DIR");
    }
    let work = certs_dir.join(format!(".enroll-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&work).await?;
    let csr = work.join("agent.csr");
    let cert = work.join("agent.crt");
    let ext = work.join("agent.ext");
    tokio::fs::write(&csr, csr_pem).await?;
    let tls_name = format!("node-{node_id}.honey");
    tokio::fs::write(
        &ext,
        format!(
            "subjectAltName=DNS:{tls_name},DNS:honey-agent\nextendedKeyUsage=serverAuth,clientAuth\nkeyUsage=digitalSignature,keyEncipherment\n"
        ),
    )
    .await?;
    let mut serial_bytes = [0u8; 16];
    getrandom::getrandom(&mut serial_bytes).map_err(|error| anyhow!("rng failed: {error}"))?;
    let serial = serial_bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();

    let result = run_openssl([
        "x509",
        "-req",
        "-in",
        path(&csr)?,
        "-CA",
        path(&ca_cert)?,
        "-CAkey",
        path(&ca_key)?,
        "-set_serial",
        &format!("0x{serial}"),
        "-days",
        "90",
        "-sha256",
        "-extfile",
        path(&ext)?,
        "-out",
        path(&cert)?,
    ])
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_dir_all(&work).await;
        return Err(error);
    }

    let fingerprint_output = run_openssl([
        "x509",
        "-in",
        path(&cert)?,
        "-noout",
        "-fingerprint",
        "-sha256",
    ])
    .await?;
    let fingerprint = fingerprint_output
        .trim()
        .split_once('=')
        .map(|(_, value)| value.replace(':', ""))
        .context("openssl returned no certificate fingerprint")?;
    let certificate_pem = tokio::fs::read_to_string(&cert).await?;
    let ca_pem = tokio::fs::read_to_string(&ca_cert).await?;
    let _ = tokio::fs::remove_dir_all(&work).await;
    let not_before = Utc::now();
    Ok(IssuedCertificate {
        certificate_pem,
        ca_pem,
        serial_number: serial,
        fingerprint_sha256: fingerprint,
        subject: format!("CN={tls_name}"),
        not_before,
        not_after: not_before + Duration::days(90),
    })
}

async fn run_openssl<'a>(args: impl IntoIterator<Item = &'a str>) -> Result<String> {
    let output = Command::new("openssl")
        .args(args)
        .output()
        .await
        .context("could not execute openssl")?;
    if !output.status.success() {
        anyhow::bail!(
            "openssl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn path(value: &PathBuf) -> Result<&str> {
    value
        .to_str()
        .ok_or_else(|| anyhow!("certificate path is not valid UTF-8"))
}
