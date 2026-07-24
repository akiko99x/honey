//! in-memory registry of live agent connections, keyed by the node's db id.
//!
//! two ways a node lands here:
//!   - serve mode: master dials the node (`connect_serve`)
//!   - dial mode:  the node dials master's acceptor, which calls `register`
//!
//! `push` applies a fresh NodeSpec unconditionally; `reconcile_push` skips it if
//! the spec is byte-identical to what was last applied (so the reconcile loop
//! doesn't restart sing-box every tick).
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use prost::Message;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio::time::Instant;
use uuid::Uuid;

use crate::agent_client::AgentClient;
use crate::auth;
use crate::db::{models::Node, repo};
use crate::pb::CoreStatus;
use crate::spec;

pub enum PushOutcome {
    Unchanged,
    Pushed,
    Deferred,
}

pub struct Registry {
    pool: PgPool,
    conns: Mutex<HashMap<Uuid, AgentClient>>,
    deferred_pushes: Mutex<HashMap<Uuid, Instant>>,
}

impl Registry {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            conns: Mutex::new(HashMap::new()),
            deferred_pushes: Mutex::new(HashMap::new()),
        }
    }

    /// Debounce a config push triggered by an API mutation. Some deployments
    /// serve the panel through an Xray fallback on the same port that an apply
    /// restarts; pushing before the HTTP response is flushed would therefore
    /// make a successful mutation look like a browser network error.
    pub async fn defer_push(self: &Arc<Self>, node_id: Uuid, delay: Duration) {
        let deadline = Instant::now() + delay;
        self.deferred_pushes.lock().await.insert(node_id, deadline);
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            let due = {
                let mut pending = registry.deferred_pushes.lock().await;
                take_due_push(&mut pending, node_id, Instant::now())
            };
            if due && registry.is_connected(node_id).await {
                let _ = registry.auto_push(node_id).await;
            }
        });
    }

    async fn push_is_deferred(&self, node_id: Uuid) -> bool {
        self.deferred_pushes.lock().await.contains_key(&node_id)
    }

    /// serve mode: dial the node at its db address, handshake, and register it.
    pub async fn connect_serve(&self, node: &Node, certs_dir: &Path) -> Result<()> {
        let endpoint = format!("https://{}:{}", node.address, node.grpc_port);
        let mut client = AgentClient::connect(&endpoint, certs_dir, &node.tls_server_name).await?;

        self.authorize_certificate(node.id, client.peer_fingerprint_sha256())
            .await?;

        tracing::debug!(code = "M0401", node = %node.name, "reaching out to node...");
        let who = client.whoru().await?;
        if who.node_id != node.id.to_string() {
            tracing::warn!(code = "M0408", node = %node.name, expected = %node.id, got = %who.node_id, "node reported the wrong id, ignoring");
            anyhow::bail!(
                "node_id mismatch: expected {}, agent reported {}",
                node.id,
                who.node_id
            );
        }
        repo::touch_node(
            &self.pool,
            node.id,
            &who.agent_version,
            &who.singbox_version,
        )
        .await?;

        self.register(node.id, client).await;
        tracing::info!(code = "M0403", node = %node.name, "node is online");
        Ok(())
    }

    /// register an already-connected agent (used by both transports).
    pub async fn register(&self, node_id: Uuid, client: AgentClient) {
        self.conns.lock().await.insert(node_id, client);
    }

    /// Enrolled nodes are pinned to the active certificate inventory in
    /// addition to ordinary CA/name validation. Inventory-free legacy nodes
    /// remain compatible until their first enrollment.
    pub async fn authorize_certificate(&self, node_id: Uuid, fingerprint: &str) -> Result<()> {
        let (has_inventory, active) =
            repo::authorize_node_certificate(&self.pool, node_id, fingerprint).await?;
        if has_inventory && !active {
            tracing::warn!(code = "M0810", %node_id, "agent certificate is revoked, expired, or not registered");
            anyhow::bail!("agent certificate is revoked, expired, or not registered");
        }
        if !has_inventory {
            tracing::warn!(code = "M0811", %node_id, "legacy CA-valid agent certificate accepted without enrollment inventory");
        }
        Ok(())
    }

    pub async fn is_connected(&self, node_id: Uuid) -> bool {
        self.conns.lock().await.contains_key(&node_id)
    }

    pub async fn connected_ids(&self) -> Vec<Uuid> {
        self.conns.lock().await.keys().copied().collect()
    }

    /// open the live stats stream for a connected node's core.
    pub async fn open_stats(
        &self,
        node_id: Uuid,
        core: crate::pb::CoreKind,
        interval_ms: u32,
    ) -> Result<tonic::Streaming<crate::pb::StatSample>> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        client.stats(core, interval_ms).await
    }

    /// Snapshot active connections from a connected node's core.
    pub async fn connections(
        &self,
        node_id: Uuid,
        core: crate::pb::CoreKind,
    ) -> Result<Vec<crate::pb::LiveConn>> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        match client.connections(core).await {
            Ok(conns) => Ok(conns),
            Err(error) => {
                self.remove(node_id).await;
                Err(error)
            }
        }
    }

    /// Coarse master<->node throughput over the control channel. Runs a 0/0 leg
    /// as the latency baseline, then an upload and a download leg of `bytes`
    /// each, and reports Mbps after subtracting that baseline.
    /// Returns (latency_ms, up_mbps, down_mbps).
    pub async fn benchmark(&self, node_id: Uuid, bytes: usize) -> Result<(f64, f64, f64)> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        let baseline = match client.benchmark_leg(0, 0).await {
            Ok(secs) => secs,
            Err(error) => {
                self.remove(node_id).await;
                return Err(error);
            }
        };
        let up_rt = client.benchmark_leg(bytes, 0).await?;
        let down_rt = client.benchmark_leg(0, bytes as u32).await?;

        let mbps = |secs: f64| {
            let net = (secs - baseline).max(0.0);
            if net <= 0.0 {
                0.0
            } else {
                (bytes as f64 * 8.0) / net / 1_000_000.0
            }
        };
        Ok((baseline * 1000.0, mbps(up_rt), mbps(down_rt)))
    }

    /// Per-core config drift verdict for a connected node against `spec`.
    pub async fn config_drift(
        &self,
        node_id: Uuid,
        spec: crate::pb::NodeSpec,
    ) -> Result<Vec<crate::pb::CoreDrift>> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        match client.config_drift(spec).await {
            Ok(cores) => Ok(cores),
            Err(error) => {
                self.remove(node_id).await;
                Err(error)
            }
        }
    }

    /// Live host metrics (cpu/mem/disk/bandwidth) from a connected node.
    pub async fn metrics(&self, node_id: Uuid) -> Result<crate::pb::MetricsReply> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        match client.metrics().await {
            Ok(m) => Ok(m),
            Err(error) => {
                self.remove(node_id).await;
                Err(error)
            }
        }
    }

    /// Close active connections by id on a node (device-limit enforcement).
    pub async fn close_connections(
        &self,
        node_id: Uuid,
        core: crate::pb::CoreKind,
        ids: Vec<String>,
    ) -> Result<u32> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        match client.close_connections(core, ids).await {
            Ok(closed) => Ok(closed),
            Err(error) => {
                self.remove(node_id).await;
                Err(error)
            }
        }
    }

    /// Fetch a finite structured log snapshot from a connected agent.
    pub async fn agent_logs(
        &self,
        node_id: Uuid,
        after_seq: u64,
        limit: u32,
    ) -> Result<Vec<crate::pb::AgentLogEntry>> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        match client.logs(after_seq, limit).await {
            Ok(entries) => Ok(entries),
            Err(error) => {
                self.remove(node_id).await;
                Err(error)
            }
        }
    }

    async fn client_of(&self, node_id: Uuid) -> Option<AgentClient> {
        self.conns.lock().await.get(&node_id).cloned()
    }

    /// Verify that a cached channel is still alive and refresh the node's
    /// presence timestamp. A failed RPC evicts the stale channel so serve-mode
    /// nodes are redialed on the next reconcile tick.
    pub async fn heartbeat(&self, node_id: Uuid) -> Result<i64> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;

        match client.ping().await {
            Ok(latency_ms) => {
                repo::mark_node_seen(&self.pool, node_id).await?;
                Ok(latency_ms)
            }
            Err(error) => {
                self.remove(node_id).await;
                Err(error)
            }
        }
    }

    pub async fn remove(&self, node_id: Uuid) {
        self.conns.lock().await.remove(&node_id);
    }

    /// force-apply the node's current spec (manual push from the api).
    pub async fn push(&self, node_id: Uuid) -> Result<CoreStatus> {
        self.push_with_context(node_id, "manual", None).await
    }

    /// Apply an automatic background change only while Auto-push is enabled.
    /// Manual API pushes deliberately bypass this switch.
    pub async fn auto_push(&self, node_id: Uuid) -> Result<Option<CoreStatus>> {
        if repo::setting_i64(&self.pool, "auto_push_enabled", 1).await == 0 {
            return Ok(None);
        }
        self.push_with_context(node_id, "automatic", None)
            .await
            .map(Some)
    }

    pub async fn push_with_context(
        &self,
        node_id: Uuid,
        source: &str,
        actor_admin_id: Option<Uuid>,
    ) -> Result<CoreStatus> {
        // An explicit/manual push supersedes any queued API mutation push.
        self.deferred_pushes.lock().await.remove(&node_id);
        let spec = spec::build_node_spec(&self.pool, node_id).await?;
        let bytes = spec.encode_to_vec();
        let desired_hash = auth::spec_hash(&bytes);
        let applied_summary = serde_json::to_value(spec::summarize(&spec))?;
        tracing::debug!(code = "M0404", %node_id, source, "pushing spec to node");
        let event =
            repo::start_node_push(&self.pool, node_id, &desired_hash, source, actor_admin_id)
                .await?;
        match self.apply(node_id, spec).await {
            Ok(status) => {
                repo::finish_node_push(
                    &self.pool,
                    event,
                    node_id,
                    &desired_hash,
                    "applied",
                    Some(&status.message),
                    Some(&applied_summary),
                )
                .await?;
                tracing::info!(code = "M0405", %node_id, state = ?status.state(), "node applied the spec");
                Ok(status)
            }
            Err(error) => {
                // Detailed agent/core errors stay in correlated logs. Persisting
                // them into API-visible node history can expose config paths or
                // credential-shaped values returned by third-party cores.
                let public_message = "push failed; inspect correlated master and agent logs";
                repo::finish_node_push(
                    &self.pool,
                    event,
                    node_id,
                    &desired_hash,
                    "failed",
                    Some(public_message),
                    None,
                )
                .await?;
                tracing::warn!(code = "M0406", %node_id, "push to node failed: {error:#}");
                crate::notify::alert(
                    &self.pool,
                    "push_failed",
                    &format!("push_failed:{node_id}"),
                    "⚠️ honey: push failed",
                    &format!("node {node_id}: {public_message}"),
                    &node_id.to_string(),
                )
                .await;
                Err(error)
            }
        }
    }

    /// Build and validate the desired candidate on the real agent without
    /// changing live process, firewall, marker, or config state.
    pub async fn dry_run(&self, node_id: Uuid) -> Result<CoreStatus> {
        let spec = spec::build_node_spec(&self.pool, node_id).await?;
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;
        match client.validate(spec).await {
            Ok(status) => Ok(status),
            Err(error) => {
                self.remove(node_id).await;
                Err(error)
            }
        }
    }

    /// apply only if the spec changed since last time (reconcile loop).
    pub async fn reconcile_push(&self, node_id: Uuid) -> Result<PushOutcome> {
        if self.push_is_deferred(node_id).await {
            return Ok(PushOutcome::Deferred);
        }
        let spec = spec::build_node_spec(&self.pool, node_id).await?;
        let bytes = spec.encode_to_vec();
        let desired_hash = auth::spec_hash(&bytes);
        let node = repo::get_node(&self.pool, node_id)
            .await?
            .ok_or_else(|| anyhow!("node {node_id} not found"))?;

        if node.applied_spec_hash.as_deref() == Some(&desired_hash) {
            return Ok(PushOutcome::Unchanged);
        }

        self.push_with_context(node_id, "reconcile", None).await?;
        Ok(PushOutcome::Pushed)
    }

    /// send an Apply to the connected agent; drop the conn if the rpc fails.
    async fn apply(&self, node_id: Uuid, spec: crate::pb::NodeSpec) -> Result<CoreStatus> {
        let mut client = self
            .client_of(node_id)
            .await
            .ok_or_else(|| anyhow!("node {node_id} is not connected"))?;

        match client.apply(spec).await {
            Ok(status) => Ok(status),
            Err(e) => {
                self.remove(node_id).await; // stale channel — force a reconnect
                tracing::warn!(code = "M0407", %node_id, "node disconnected!");
                Err(e)
            }
        }
    }
}

fn take_due_push(pending: &mut HashMap<Uuid, Instant>, node_id: Uuid, now: Instant) -> bool {
    match pending.get(&node_id).copied() {
        Some(deadline) if deadline <= now => {
            pending.remove(&node_id);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_push_waits_for_the_latest_deadline() {
        let node_id = Uuid::new_v4();
        let now = Instant::now();
        let mut pending = HashMap::new();
        pending.insert(node_id, now + Duration::from_secs(5));

        assert!(!take_due_push(&mut pending, node_id, now));
        assert!(pending.contains_key(&node_id));

        assert!(take_due_push(
            &mut pending,
            node_id,
            now + Duration::from_secs(5)
        ));
        assert!(!pending.contains_key(&node_id));
    }
}
