//! Rolling quota windows: reset a user's usage at daily / weekly boundaries so
//! `traffic_limit_bytes` applies per window instead of for all time.
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::PgPool;

use crate::db::repo;
use crate::registry::Registry;

pub async fn run(pool: PgPool, registry: Arc<Registry>, tick: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(
        code = "M1400",
        secs = tick.as_secs(),
        "quota window scheduler up"
    );
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        let due = match repo::due_quota_resets(&pool).await {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(code = "M1402", "quota scan failed: {e:#}");
                continue;
            }
        };
        for (user_id, interval) in due {
            let nodes = repo::user_node_ids(&pool, user_id)
                .await
                .unwrap_or_default();
            if let Err(e) = repo::reset_user_traffic(&pool, user_id).await {
                tracing::warn!(code = "M1402", %user_id, "quota reset failed: {e:#}");
                continue;
            }
            let _ = repo::advance_quota_reset(&pool, user_id, next_boundary(&interval)).await;
            for id in nodes {
                if registry.is_connected(id).await {
                    let _ = registry.auto_push(id).await;
                }
            }
            tracing::info!(code = "M1401", %user_id, interval = %interval, "quota window reset");
            crate::notify::alert(
                &pool,
                "quota_reset",
                &format!("quota_reset:{user_id}:{}", chrono::Utc::now().date_naive()),
                "🔄 honey: quota reset",
                &format!("user {user_id} — {interval} traffic window reset"),
                &user_id.to_string(),
            )
            .await;
        }
    }
}

pub fn next_boundary(interval: &str) -> DateTime<Utc> {
    let days = if interval == "weekly" { 7 } else { 1 };
    Utc::now() + ChronoDuration::days(days)
}
