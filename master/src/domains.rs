//! Verification for managed domains: resolve DNS, probe :443 reachability, and
//! read the served TLS certificate's expiry.
//!
//! For a CDN-fronted (proxied) domain the resolved IPs are the CDN's, so we
//! can't expect them to match the node — we only require that it resolves; the
//! cert we read on :443 is still the one clients see. For a direct domain tied
//! to a node we check the node IP is among the answers. This is a signal, not a
//! guarantee: CDN hides the origin IP but does not prove the domain/SNI/CDN
//! subnet won't be blocked.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use tokio::net::{lookup_host, TcpStream};
use uuid::Uuid;

use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, SignatureScheme,
};
use tokio_rustls::TlsConnector;

use crate::db::{models::ManagedDomain, repo};

const RENEW_WARN_DAYS: i64 = 14;

pub struct Verdict {
    pub dns_ok: bool,
    pub resolved_ips: Vec<String>,
    pub reachable_443: bool,
    pub cert_not_after: Option<DateTime<Utc>>,
    pub cert_ok: bool,
    pub error: Option<String>,
}

pub async fn verify(host: &str, proxied: bool, expected_ip: Option<&str>) -> Verdict {
    let mut error: Option<String> = None;

    // --- DNS -----------------------------------------------------------------
    let mut ips = std::collections::BTreeSet::new();
    match lookup_host((host, 443u16)).await {
        Ok(addrs) => {
            for addr in addrs {
                ips.insert(addr.ip().to_string());
            }
        }
        Err(e) => error = Some(format!("dns: {e}")),
    }
    let resolved_ips: Vec<String> = ips.into_iter().collect();
    let dns_ok = if resolved_ips.is_empty() {
        false
    } else if proxied {
        true
    } else if let Some(expected) = expected_ip {
        resolved_ips.iter().any(|ip| ip == expected)
    } else {
        true
    };
    if !dns_ok && error.is_none() && !resolved_ips.is_empty() {
        error = Some(format!(
            "resolves to {} but not the node address",
            resolved_ips.join(", ")
        ));
    }

    // --- :443 reachability ---------------------------------------------------
    let reachable_443 = match tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect((host, 443u16)),
    )
    .await
    {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            if error.is_none() {
                error = Some(format!("tcp :443: {e}"));
            }
            false
        }
        Err(_) => {
            if error.is_none() {
                error = Some("tcp :443: timed out".to_string());
            }
            false
        }
    };

    // --- TLS certificate expiry ---------------------------------------------
    let (cert_not_after, cert_ok, cert_err) = if reachable_443 {
        probe_cert(host).await
    } else {
        (None, false, None)
    };
    if let Some(e) = cert_err {
        if error.is_none() {
            error = Some(e);
        }
    }

    Verdict {
        dns_ok,
        resolved_ips,
        reachable_443,
        cert_not_after,
        cert_ok,
        error,
    }
}

/// Run a full check on one domain and persist the verdict. Warns if the cert is
/// close to expiry. Shared by the API verify handler and the background monitor.
pub async fn run_and_store(pool: &PgPool, id: Uuid) -> Result<Option<ManagedDomain>> {
    let Some(domain) = repo::get_managed_domain(pool, id).await? else {
        return Ok(None);
    };
    let expected_ip = match domain.node_id {
        Some(node_id) => repo::get_node(pool, node_id).await?.map(|n| n.address),
        None => None,
    };
    let v = verify(&domain.host, domain.proxied, expected_ip.as_deref()).await;

    if let Some(not_after) = v.cert_not_after {
        let days = (not_after - Utc::now()).num_days();
        if days <= RENEW_WARN_DAYS {
            tracing::warn!(code = "M1301", domain = %domain.host, days, "domain certificate expires soon");
            crate::notify::alert(
                pool,
                "cert_expiry",
                &format!("cert_expiry:{}", domain.host),
                "🔒 honey: certificate expiring",
                &format!("{} expires in {days} day(s)", domain.host),
                &domain.id.to_string(),
            )
            .await;
        }
    }

    repo::set_managed_domain_check(
        pool,
        id,
        v.dns_ok,
        &v.resolved_ips,
        v.reachable_443,
        v.cert_not_after,
        v.cert_ok,
        v.error.as_deref(),
    )
    .await
}

/// Periodically re-verify every managed domain so cert-expiry warnings and DNS
/// drift surface without a manual click.
pub async fn monitor(pool: PgPool, interval: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(
        code = "M1300",
        secs = interval.as_secs(),
        "domain monitor up"
    );
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        let ids = match repo::list_managed_domains(&pool).await {
            Ok(domains) => domains.into_iter().map(|d| d.id).collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(code = "M1302", "domain monitor: list failed: {e:#}");
                continue;
            }
        };
        for id in ids {
            if let Err(e) = run_and_store(&pool, id).await {
                tracing::debug!(code = "M1302", %id, "domain check failed: {e:#}");
            }
        }
    }
}

async fn probe_cert(host: &str) -> (Option<DateTime<Utc>>, bool, Option<String>) {
    let tcp = match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect((host, 443u16)))
        .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return (None, false, Some(format!("tls connect: {e}"))),
        Err(_) => return (None, false, Some("tls connect: timed out".into())),
    };
    let server_name = match ServerName::try_from(host.to_string()) {
        Ok(n) => n,
        Err(_) => return (None, false, Some("invalid tls server name".into())),
    };
    let connector = TlsConnector::from(client_config());
    let tls =
        match tokio::time::timeout(Duration::from_secs(5), connector.connect(server_name, tcp))
            .await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return (None, false, Some(format!("tls handshake: {e}"))),
            Err(_) => return (None, false, Some("tls handshake: timed out".into())),
        };
    let (_io, conn) = tls.get_ref();
    let Some(leaf) = conn.peer_certificates().and_then(|c| c.first()) else {
        return (None, false, Some("no peer certificate".into()));
    };
    match x509_parser::parse_x509_certificate(leaf.as_ref()) {
        Ok((_, cert)) => {
            let not_after = Utc
                .timestamp_opt(cert.validity().not_after.timestamp(), 0)
                .single();
            let cert_ok = not_after.map(|na| na > Utc::now()).unwrap_or(false);
            (not_after, cert_ok, None)
        }
        Err(e) => (None, false, Some(format!("cert parse: {e}"))),
    }
}

fn client_config() -> Arc<ClientConfig> {
    let provider = tokio_rustls::rustls::crypto::ring::default_provider();
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny))
        .with_no_client_auth();
    Arc::new(config)
}

/// A probe reads the served certificate but does not trust-verify it — an
/// expired or self-signed cert is exactly what we want to observe.
#[derive(Debug)]
struct AcceptAny;

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}
