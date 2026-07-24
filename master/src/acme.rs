//! Built-in ACME (feature `acme`): obtains and auto-renews certs over
//! TLS-ALPN-01 on the listen port (must be 443, reachable from the internet).
//!
//! This suits a master on its own domain. On a single-server box where REALITY
//! already holds 443, use `--tls-cert/--tls-key` with an external ACME client
//! instead (see https.rs). Version-sensitive (rustls-acme <-> axum-server); kept
//! behind the `acme` feature so it can't affect other builds.
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::Router;
use futures_util::StreamExt;
use rustls_acme::{caches::DirCache, AcmeConfig};

pub async fn serve(
    addr: SocketAddr,
    app: Router,
    domains: Vec<String>,
    email: String,
    cache: PathBuf,
    staging: bool,
) -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let domain_count = domains.len();
    let mut state = AcmeConfig::new(domains)
        .contact_push(format!("mailto:{email}"))
        .cache(DirCache::new(cache))
        .directory_lets_encrypt(!staging) // true = production
        .state();

    let acceptor = state.axum_acceptor(state.default_rustls_config());

    // drive the ACME order/renewal loop.
    tokio::spawn(async move {
        loop {
            match state.next().await {
                Some(Ok(ok)) => tracing::info!(code = "M0903", "acme: {ok:?}"),
                Some(Err(err)) => tracing::error!(code = "M0904", "acme error: {err}"),
                None => break,
            }
        }
    });

    tracing::info!(code = "M0104", %addr, domains = domain_count, staging, "rest api up (https+acme)");
    axum_server::bind(addr)
        .acceptor(acceptor)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("acme https server error")
}
