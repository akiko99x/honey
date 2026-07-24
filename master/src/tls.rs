use std::io::BufReader;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// Build the mTLS client config used by the master for serve-mode nodes.
/// The caller keeps ownership of the resulting TLS stream so the exact peer
/// certificate carried by the gRPC channel can be authorization-checked.
pub fn rustls_client_config(certs_dir: &Path) -> Result<rustls::ClientConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    for cert in load_certs(&certs_dir.join("ca.crt"))? {
        roots.add(cert).context("add honey CA certificate")?;
    }
    let identity = load_certs(&certs_dir.join("master.crt"))?;
    let key = load_key(&certs_dir.join("master.key"))?;
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(identity, key)
        .context("build master mTLS client config")?;
    Ok(with_grpc_alpn(config))
}

/// Pre-negotiated TLS streams still need to advertise HTTP/2 before tonic
/// takes ownership of them. Without this ALPN value the mTLS handshake
/// succeeds, but the gRPC transport immediately closes the one-shot stream.
pub(crate) fn with_grpc_alpn(mut config: rustls::ClientConfig) -> rustls::ClientConfig {
    config.alpn_protocols = vec![b"h2".to_vec()];
    config
}

pub fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut reader = BufReader::new(&data[..]);
    let certs = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
    if certs.is_empty() {
        anyhow::bail!("{} contains no certificates", path.display());
    }
    Ok(certs)
}

pub fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut reader = BufReader::new(&data[..]);
    rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| anyhow!("no private key in {}", path.display()))
}

/// Canonical uppercase SHA-256, matching the OpenSSL-normalized fingerprint
/// stored by enrollment.
pub fn certificate_fingerprint(certificate_der: &[u8]) -> String {
    Sha256::digest(certificate_der)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_uppercase_sha256() {
        assert_eq!(
            certificate_fingerprint(b"honey"),
            "A55E2E3846A51F6AD0ABFDFBDEA2BA0E5E0C76B5CCFA8A920895FEDEAE89A8B6"
        );
    }

    #[test]
    fn grpc_clients_offer_http2_alpn() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let config = with_grpc_alpn(config);
        assert_eq!(config.alpn_protocols, vec![b"h2".to_vec()]);
    }
}
