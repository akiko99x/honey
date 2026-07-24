//! dial-mode acceptor: nodes behind NAT dial IN to us, and we drive them back
//! as a grpc client over that same accepted socket. see docs/transports.md.
//!
//! NOTE: this is the one place that wires tonic 0.12 over an already-accepted
//! TLS stream, so it's feature-gated (`dial-acceptor`) and its extra deps are
//! optional. it is the module most likely to need small version tweaks on the
//! first real `cargo build --features dial-acceptor`.
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use sqlx::PgPool;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_rustls::TlsConnector;
use tonic::transport::{Endpoint, Uri};
use tower::service_fn;
use uuid::Uuid;

use crate::agent_client::AgentClient;
use crate::db::repo;
use crate::registry::Registry;

pub async fn run(
    pool: PgPool,
    registry: Arc<Registry>,
    certs_dir: PathBuf,
    listen: &str,
) -> Result<()> {
    // rustls 0.23 needs a process-wide crypto provider installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let tls = Arc::new(client_config(&certs_dir)?);
    let listener = TcpListener::bind(listen)
        .await
        .with_context(|| format!("bind {listen}"))?;
    tracing::info!(%listen, "dial acceptor up");

    loop {
        let (stream, peer) = listener.accept().await?;
        let tls = tls.clone();
        let registry = registry.clone();
        let pool = pool.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, tls, &pool, &registry).await {
                tracing::warn!("dial-in from {peer} failed: {e:#}");
            }
        });
    }
}

async fn handle(
    stream: TcpStream,
    tls: Arc<rustls::ClientConfig>,
    pool: &PgPool,
    registry: &Arc<Registry>,
) -> Result<()> {
    // the agent dialed us, but above the socket we are still the TLS client and
    // the grpc client — the agent stays the AgentService server.
    let server_name = ServerName::try_from("honey-agent")
        .expect("static name")
        .to_owned();
    let tls_stream = TlsConnector::from(tls)
        .connect(server_name, stream)
        .await
        .context("tls client handshake")?;
    let peer_fingerprint_sha256 = tls_stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .map(|certificate| crate::tls::certificate_fingerprint(certificate.as_ref()))
        .context("dial agent TLS peer sent no certificate")?;

    // hand the single, already-open stream to tonic as its transport. the
    // connector yields it once; a reconnect (there won't be one) errors out.
    let slot = Arc::new(Mutex::new(Some(tls_stream)));
    let connector = service_fn(move |_: Uri| {
        let slot = slot.clone();
        async move {
            match slot.lock().await.take() {
                Some(s) => Ok::<_, std::io::Error>(TokioIo::new(s)),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "tunnel already consumed",
                )),
            }
        }
    });

    let channel = Endpoint::from_static("http://honey-agent")
        .connect_with_connector(connector)
        .await
        .context("grpc over tunnel")?;

    let mut client = AgentClient::from_channel(channel, peer_fingerprint_sha256);
    let who = client.whoru().await?;

    // convention: the agent's --node-id is the node's db uuid.
    let node_id = Uuid::parse_str(&who.node_id).map_err(|_| {
        anyhow!(
            "agent node_id '{}' is not a db uuid — start the agent with --node-id = db id",
            who.node_id
        )
    })?;

    let node = repo::get_node(pool, node_id)
        .await?
        .ok_or_else(|| anyhow!("agent node_id '{node_id}' does not exist in the database"))?;
    if !node.enabled {
        anyhow::bail!("node '{node_id}' is disabled");
    }
    if !matches!(node.transport.as_str(), "dial" | "both") {
        anyhow::bail!(
            "node '{node_id}' is configured for '{}' transport, not dial",
            node.transport
        );
    }
    registry
        .authorize_certificate(node_id, client.peer_fingerprint_sha256())
        .await?;
    repo::touch_node(pool, node_id, &who.agent_version, &who.singbox_version).await?;
    registry.register(node_id, client).await;

    tracing::info!(node_id = %node_id, host = %who.hostname, "node dialed in, registered");
    Ok(())
}

fn client_config(certs_dir: &Path) -> Result<rustls::ClientConfig> {
    let ca = load_certs(&certs_dir.join("ca.crt"))?;
    let mut roots = rustls::RootCertStore::empty();
    for c in ca {
        roots.add(c).context("add ca cert")?;
    }

    let cert = load_certs(&certs_dir.join("master.crt"))?;
    let key = load_key(&certs_dir.join("master.key"))?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(cert, key)
        .context("build client tls")?;
    Ok(crate::tls::with_grpc_alpn(config))
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut rd = BufReader::new(&data[..]);
    let certs = rustls_pemfile::certs(&mut rd).collect::<Result<Vec<_>, _>>()?;
    Ok(certs)
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut rd = BufReader::new(&data[..]);
    rustls_pemfile::private_key(&mut rd)?
        .ok_or_else(|| anyhow!("no private key in {}", path.display()))
}
