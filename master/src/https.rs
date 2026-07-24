//! Built-in HTTPS for the api/panel (feature `tls`). Terminates TLS from a
//! cert/key pair on disk and hot-reloads it hourly, so certs renewed by an
//! external ACME client (certbot / acme.sh, via HTTP-01 or DNS-01) are picked
//! up without a restart. This fits single-server nodes where port 443 is held
//! by REALITY and in-process TLS-ALPN ACME can't run.
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;

pub async fn serve(addr: SocketAddr, app: Router, cert: PathBuf, key: PathBuf) -> Result<()> {
    // rustls 0.23 needs a process crypto provider; harmless if already set.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = RustlsConfig::from_pem_file(&cert, &key)
        .await
        .with_context(|| format!("load tls cert {} / key {}", cert.display(), key.display()))?;

    // reload renewed certs without a restart.
    let reloader = config.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            match reloader.reload_from_pem_file(&cert, &key).await {
                Ok(()) => tracing::info!(code = "M0901", "tls certificate reloaded"),
                Err(error) => {
                    tracing::warn!(code = "M0902", %error, "tls certificate reload failed")
                }
            }
        }
    });

    tracing::info!(code = "M0103", %addr, "rest api up (https)");
    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context("https server error")
}
