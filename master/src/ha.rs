//! Multi-master HA: Postgres-lease leader election.
//!
//! Every instance serves the API (sessions and state live in the database), but
//! the singleton background loops — reconcile, stats, quota, schedule, the
//! monitors, ACME and the Telegram bot — must run on exactly one instance or
//! they would double-push, double-count and duplicate alerts. This module elects
//! that instance.
//!
//! Safety model: the lease has a TTL and the holder renews it well before it
//! expires. A holder that *fails* to renew (database unreachable, partition)
//! immediately steps down locally, so it stops acting before another instance
//! can take over — the takeover can only happen after the lease actually
//! expires. A single-instance deployment simply always wins the election, so
//! nothing changes for non-HA setups.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::repo;

static INSTANCE_ID: OnceLock<Uuid> = OnceLock::new();
static IS_LEADER: AtomicBool = AtomicBool::new(false);

/// This process's stable identity for the lease and the instance roster.
pub fn instance_id() -> Uuid {
    *INSTANCE_ID.get_or_init(Uuid::new_v4)
}

/// Whether this instance currently holds the leader lease. Background loops
/// gate their work on this.
pub fn is_leader() -> bool {
    IS_LEADER.load(Ordering::Relaxed)
}

fn set_leader(value: bool) {
    let previous = IS_LEADER.swap(value, Ordering::Relaxed);
    if previous != value {
        if value {
            tracing::info!(
                code = "M0120",
                instance = %instance_id(),
                "became leader — running singleton background loops"
            );
        } else {
            tracing::warn!(
                code = "M0121",
                instance = %instance_id(),
                "lost leadership — background loops paused on this instance"
            );
        }
    }
}

/// Run the election. `ttl` is how long a lease survives without renewal (the
/// worst-case failover delay); renewal happens at a third of that.
pub async fn run(pool: PgPool, ttl: Duration) -> Result<()> {
    let ttl_secs = ttl.as_secs().max(3) as i64;
    let renew = Duration::from_secs((ttl_secs as u64 / 3).max(1));
    let hostname = hostname();
    let version = env!("CARGO_PKG_VERSION").to_string();
    tracing::info!(
        code = "M0119",
        instance = %instance_id(),
        "ha election up (lease ttl {ttl_secs}s)"
    );

    let mut ticker = tokio::time::interval(renew);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ticks: u64 = 0;
    loop {
        ticker.tick().await;
        match repo::ha_try_acquire(&pool, instance_id(), ttl_secs).await {
            Ok(held) => set_leader(held),
            Err(error) => {
                // cannot reach the database: step down rather than risk acting
                // while another instance takes the expired lease.
                set_leader(false);
                tracing::warn!(code = "M0121", "ha lease renew failed: {error:#}");
            }
        }
        if let Err(error) = repo::ha_heartbeat(&pool, instance_id(), &hostname, &version).await {
            tracing::debug!(code = "M0121", "ha heartbeat failed: {error:#}");
        }
        ticks = ticks.wrapping_add(1);
        // the leader prunes instances that stopped heartbeating.
        if is_leader() && ticks % 10 == 0 {
            if let Err(error) = repo::ha_prune_instances(&pool, ttl_secs * 6).await {
                tracing::debug!(code = "M0121", "ha instance prune failed: {error:#}");
            }
        }
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_id_is_stable() {
        assert_eq!(instance_id(), instance_id());
    }

    #[test]
    fn leadership_flag_round_trips() {
        set_leader(true);
        assert!(is_leader());
        set_leader(false);
        assert!(!is_leader());
    }
}
