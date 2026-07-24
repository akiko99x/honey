//! background loop that keeps agents converged to the db's desired state.
//! each tick: make sure serve-mode nodes are connected, then push their spec
//! (only if it changed — see Registry::reconcile_push).
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;

use crate::db::repo;
use crate::registry::{PushOutcome, Registry};

pub async fn run(
    pool: PgPool,
    registry: Arc<Registry>,
    certs_dir: PathBuf,
    interval: Duration,
) -> Result<()> {
    // the cadence is runtime-editable via the `reconcile_secs` setting; the CLI
    // value is the fallback when it is unset. re-read every tick so a panel edit
    // takes effect on the next cycle without a restart.
    let default_secs = interval.as_secs().max(1) as i64;
    tracing::info!(
        code = "M0106",
        secs = default_secs,
        "reconcile loop up, watching for drift"
    );

    loop {
        // HA: only the lease holder converges nodes, or instances would fight
        // over pushes.
        if crate::ha::is_leader() {
            if let Err(e) = tick(&pool, &registry, &certs_dir).await {
                tracing::warn!(code = "M0501", "reconcile tick tripped: {e:#}");
            }
        }
        let secs = repo::setting_i64(&pool, "reconcile_secs", default_secs)
            .await
            .clamp(5, 86_400) as u64;
        tokio::time::sleep(Duration::from_secs(secs)).await;
    }
}

async fn tick(pool: &PgPool, registry: &Arc<Registry>, certs_dir: &Path) -> Result<()> {
    let auto_push_enabled = repo::setting_i64(pool, "auto_push_enabled", 1).await != 0;
    for node in repo::list_nodes(pool).await? {
        if !node.enabled {
            continue;
        }

        // serve/both: master dials the node. dial: wait for it to come to us.
        if node.transport != "dial" && !registry.is_connected(node.id).await {
            if let Err(e) = registry.connect_serve(&node, certs_dir).await {
                tracing::debug!(code = "M0502", node = %node.name, "reconcile: node still unreachable: {e:#}");
            }
        }

        if registry.is_connected(node.id).await {
            if let Err(e) = registry.heartbeat(node.id).await {
                tracing::warn!(
                    code = "M0409",
                    node = %node.name,
                    "node went down (heartbeat failed): {e:#}"
                );
                crate::notify::alert(
                    pool,
                    "node_down",
                    &format!("node_down:{}", node.id),
                    "🔴 honey: node down",
                    &format!("{} ({}) is unreachable", node.name, node.address),
                    &node.id.to_string(),
                )
                .await;
                continue;
            }

            // push logging (M0404/M0405/M0406) lives in Registry::push_with_context,
            // so it covers manual, reconcile and quota-triggered pushes alike.
            if auto_push_enabled {
                match registry.reconcile_push(node.id).await {
                    Ok(PushOutcome::Pushed)
                    | Ok(PushOutcome::Unchanged)
                    | Ok(PushOutcome::Deferred) => {}
                    Err(e) => tracing::debug!(node = %node.name, "reconcile: {e:#}"),
                }
            }
        }
    }
    Ok(())
}
