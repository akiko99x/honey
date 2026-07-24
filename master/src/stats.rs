//! stats collector: keeps a live Stats stream open per (connected node, core)
//! and records per-user traffic into the db. one subscription per (node, core);
//! a dropped stream is retried on the next poll tick.
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::db::repo;
use crate::pb::CoreKind;
use crate::registry::Registry;

// cores we pull stats from. xray only reports if it's running with inbounds;
// otherwise the stream errors and is retried harmlessly.
const CORES: [CoreKind; 2] = [CoreKind::Singbox, CoreKind::Xray];
pub const DEFAULT_TRAFFIC_HISTORY_DAYS: i64 = 180;
const MAX_TRAFFIC_HISTORY_DAYS: i64 = 3650;

pub async fn run(
    pool: PgPool,
    registry: Arc<Registry>,
    poll: Duration,
    sample_ms: u32,
) -> Result<()> {
    let active: Arc<Mutex<HashSet<(Uuid, i32)>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut ticker = tokio::time::interval(poll);
    tracing::info!(code = "M0107", "stats collector up");

    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        for node_id in registry.connected_ids().await {
            for core in CORES {
                let key = (node_id, core as i32);
                // one subscription per (node, core).
                {
                    let mut guard = active.lock().await;
                    if !guard.insert(key) {
                        continue;
                    }
                }

                let pool = pool.clone();
                let registry = registry.clone();
                let active = active.clone();
                tokio::spawn(async move {
                    if let Err(e) = subscribe(&pool, &registry, node_id, core, sample_ms).await {
                        tracing::debug!(code = "M0601", node = %node_id, core = core_str(core), "stats stream ended: {e:#}");
                    }
                    active.lock().await.remove(&key);
                });
            }
        }
    }
}

async fn subscribe(
    pool: &PgPool,
    registry: &Arc<Registry>,
    node_id: Uuid,
    core: CoreKind,
    sample_ms: u32,
) -> Result<()> {
    let mut stream = registry.open_stats(node_id, core, sample_ms).await?;
    let core_name = core_str(core);

    while let Some(sample) = stream.message().await? {
        let epoch = if sample.epoch.is_empty() {
            "legacy"
        } else {
            &sample.epoch
        };
        let mut quota_crossed = false;
        for user_stat in sample.users {
            if let Some(user) = repo::get_user_by_name(pool, &user_stat.name).await? {
                let up =
                    i64::try_from(user_stat.up_bytes).context("user upload counter overflow")?;
                let down = i64::try_from(user_stat.down_bytes)
                    .context("user download counter overflow")?;
                quota_crossed |=
                    repo::record_traffic(pool, node_id, user.id, core_name, epoch, up, down)
                        .await?;
            }
        }
        if quota_crossed {
            // Remove newly over-quota users immediately. A failed push drops the
            // stale connection and the normal reconcile loop retries later.
            registry.auto_push(node_id).await?;
        }
    }
    Ok(())
}

fn core_str(core: CoreKind) -> &'static str {
    match core {
        CoreKind::Xray => "xray",
        _ => "singbox",
    }
}

fn bounded_history_days(value: i64) -> i64 {
    value.clamp(7, MAX_TRAFFIC_HISTORY_DAYS)
}

/// Delete expired hourly buckets on a quiet cadence. The setting is read each
/// iteration, so an owner can change retention without restarting the master.
pub async fn retention(pool: PgPool, interval: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    tracing::info!(
        code = "M0610",
        default_days = DEFAULT_TRAFFIC_HISTORY_DAYS,
        "traffic history retention up"
    );
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        let days = bounded_history_days(
            repo::setting_i64(&pool, "traffic_history_days", DEFAULT_TRAFFIC_HISTORY_DAYS).await,
        );
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
        match repo::delete_traffic_history_before(&pool, cutoff).await {
            Ok(deleted) if deleted > 0 => tracing::info!(
                code = "M0611",
                deleted,
                days,
                "expired traffic history deleted"
            ),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                code = "M0612",
                days,
                "traffic history retention failed: {error:#}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_history_days, DEFAULT_TRAFFIC_HISTORY_DAYS};

    #[test]
    fn traffic_retention_is_safely_bounded() {
        assert_eq!(bounded_history_days(0), 7);
        assert_eq!(bounded_history_days(DEFAULT_TRAFFIC_HISTORY_DAYS), 180);
        assert_eq!(bounded_history_days(100_000), 3650);
    }
}
