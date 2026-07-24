use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use tonic::transport::Uri;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;
use url::Url;

use tonic::Streaming;

use crate::pb::agent_service_client::AgentServiceClient;
use crate::pb::{
    AgentLogEntry, AgentLogsRequest, ApplyRequest, BenchmarkRequest, CloseConnectionsRequest,
    ConfigDriftRequest, ConnectionsRequest, CoreDrift, CoreKind, CoreState, CoreStatus, LiveConn,
    MetricsReply, MetricsRequest, NodeIdentity, NodeSpec, PingRequest, StatSample, StatsRequest,
    WhoRuRequest,
};
use crate::tls;

/// a thin master-side handle to one node agent.
/// clone is cheap (the tonic channel is Arc-backed) — used so the registry can
/// hand out a client without holding its lock across an rpc.
#[derive(Clone)]
pub struct AgentClient {
    inner: AgentServiceClient<Channel>,
    peer_fingerprint_sha256: String,
}

impl AgentClient {
    /// dials an agent over mTLS. `endpoint` like "https://203.0.113.10:8443".
    pub async fn connect(endpoint: &str, certs_dir: &Path, tls_server_name: &str) -> Result<Self> {
        let endpoint_url = Url::parse(endpoint).context("bad endpoint")?;
        let host = endpoint_url
            .host_str()
            .context("agent endpoint has no host")?;
        let port = endpoint_url
            .port_or_known_default()
            .context("agent endpoint has no port")?;
        let tcp = TcpStream::connect((host, port))
            .await
            .context("connect to agent tcp endpoint")?;
        let server_name = ServerName::try_from(tls_server_name.to_string())
            .map_err(|_| anyhow!("invalid agent TLS server name"))?;
        let tls_stream = TlsConnector::from(Arc::new(tls::rustls_client_config(certs_dir)?))
            .connect(server_name, tcp)
            .await
            .context("agent mTLS handshake")?;
        let peer_fingerprint_sha256 = tls_stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .map(|certificate| tls::certificate_fingerprint(certificate.as_ref()))
            .context("agent TLS peer sent no certificate")?;

        // Use this exact authenticated TLS stream for gRPC. A separate
        // preflight connection would leave a certificate-swap race.
        let slot = Arc::new(Mutex::new(Some(tls_stream)));
        let connector = service_fn(move |_: Uri| {
            let slot = slot.clone();
            async move {
                match slot.lock().await.take() {
                    Some(stream) => Ok::<_, std::io::Error>(TokioIo::new(stream)),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "agent TLS stream already consumed",
                    )),
                }
            }
        });
        let channel = Endpoint::from_static("http://honey-agent")
            .connect_with_connector(connector)
            .await
            .context("grpc over authenticated agent stream")?;

        Ok(Self {
            inner: AgentServiceClient::new(channel),
            peer_fingerprint_sha256,
        })
    }

    /// wraps an already-connected channel (used by the dial-mode acceptor).
    #[cfg(feature = "dial-acceptor")]
    pub fn from_channel(channel: Channel, peer_fingerprint_sha256: String) -> Self {
        Self {
            inner: AgentServiceClient::new(channel),
            peer_fingerprint_sha256,
        }
    }

    pub fn peer_fingerprint_sha256(&self) -> &str {
        &self.peer_fingerprint_sha256
    }

    /// who r u — handshake, returns the node's identity.
    pub async fn whoru(&mut self) -> Result<NodeIdentity> {
        let resp = self.inner.who_ru(WhoRuRequest {}).await?;
        Ok(resp.into_inner())
    }

    /// ping — returns round-trip latency in millis.
    pub async fn ping(&mut self) -> Result<i64> {
        let sent = now_millis();
        let resp = self.inner.ping(PingRequest { sent_at: sent }).await?;
        let _ = resp.into_inner();
        Ok(now_millis() - sent)
    }

    /// apply — push a NodeSpec; the agent builds config.json and (re)starts sing-box.
    pub async fn apply(&mut self, spec: NodeSpec) -> Result<CoreStatus> {
        let req = ApplyRequest {
            core: CoreKind::Singbox as i32,
            spec: Some(spec),
            raw_config_json: String::new(),
        };
        let resp = self.inner.apply(req).await?;
        ensure_apply_success(resp.into_inner())
    }

    /// validate — ask the agent to build/check a candidate without applying it.
    pub async fn validate(&mut self, spec: NodeSpec) -> Result<CoreStatus> {
        let req = ApplyRequest {
            core: CoreKind::Singbox as i32,
            spec: Some(spec),
            raw_config_json: String::new(),
        };
        let resp = self.inner.validate(req).await?;
        Ok(resp.into_inner())
    }

    /// stats — open the server stream of live per-user + node traffic.
    pub async fn stats(
        &mut self,
        core: CoreKind,
        interval_ms: u32,
    ) -> Result<Streaming<StatSample>> {
        let req = StatsRequest {
            core: core as i32,
            interval_ms,
        };
        let resp = self.inner.stats(req).await?;
        Ok(resp.into_inner())
    }

    /// connections — one point-in-time snapshot of active connections.
    pub async fn connections(&mut self, core: CoreKind) -> Result<Vec<LiveConn>> {
        let req = ConnectionsRequest { core: core as i32 };
        let resp = self.inner.connections(req).await?;
        Ok(resp.into_inner().conns)
    }

    /// close_connections — close active connections by id (device-limit enforcement).
    pub async fn close_connections(&mut self, core: CoreKind, ids: Vec<String>) -> Result<u32> {
        let req = CloseConnectionsRequest {
            core: core as i32,
            ids,
        };
        let resp = self.inner.close_connections(req).await?;
        Ok(resp.into_inner().closed)
    }

    /// metrics — one live host snapshot (cpu/mem/disk/bandwidth).
    pub async fn metrics(&mut self) -> Result<MetricsReply> {
        let resp = self.inner.metrics(MetricsRequest {}).await?;
        Ok(resp.into_inner())
    }

    /// One benchmark leg: send `up_bytes`, ask for `down_bytes` back, and return
    /// the round-trip seconds. Callers time a 0/0 leg as the latency baseline and
    /// subtract it, which keeps the result free of master/agent clock skew.
    pub async fn benchmark_leg(&mut self, up_bytes: usize, down_bytes: u32) -> Result<f64> {
        let payload = vec![7u8; up_bytes];
        let started = std::time::Instant::now();
        self.inner
            .benchmark(BenchmarkRequest {
                payload,
                respond_bytes: down_bytes,
            })
            .await?;
        Ok(started.elapsed().as_secs_f64())
    }

    /// config_drift — per-core comparison of built-from-spec vs on-disk config.
    pub async fn config_drift(&mut self, spec: NodeSpec) -> Result<Vec<CoreDrift>> {
        let resp = self
            .inner
            .config_drift(ConfigDriftRequest { spec: Some(spec) })
            .await?;
        Ok(resp.into_inner().cores)
    }

    /// logs — collect one finite snapshot from the agent's structured ring.
    pub async fn logs(&mut self, after_seq: u64, limit: u32) -> Result<Vec<AgentLogEntry>> {
        let req = AgentLogsRequest { after_seq, limit };
        let mut stream = self.inner.logs(req).await?.into_inner();
        let mut entries = Vec::new();
        while let Some(entry) = stream.message().await? {
            entries.push(entry);
        }
        Ok(entries)
    }
}

fn ensure_apply_success(status: CoreStatus) -> Result<CoreStatus> {
    if status.state() == CoreState::Errored {
        let message = if status.message.is_empty() {
            "agent reported an errored core".to_string()
        } else {
            status.message.clone()
        };
        anyhow::bail!("agent apply failed: {message}");
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errored_apply_status_is_an_error() {
        let status = CoreStatus {
            state: CoreState::Errored as i32,
            message: "bad candidate".into(),
            ..Default::default()
        };
        let error = ensure_apply_success(status).unwrap_err().to_string();
        assert!(error.contains("bad candidate"));
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
