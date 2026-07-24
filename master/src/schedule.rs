//! Scheduler: runs deferred operations (enable/disable/push/…) at their due
//! time. Each tick claims due pending rows, executes them and records the
//! outcome. Safe to run alongside the reconcile loop — actions are idempotent.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::models::ScheduledOp;
use crate::db::repo;
use crate::registry::Registry;

pub async fn run(pool: PgPool, registry: Arc<Registry>, tick: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(code = "M1800", secs = tick.as_secs(), "scheduler up");
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        let due = match repo::due_scheduled_ops(&pool).await {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(code = "M1801", "scheduler scan failed: {error:#}");
                continue;
            }
        };
        for op in due {
            let (status, result) = match execute(&pool, &registry, &op).await {
                Ok(message) => ("done", message),
                Err(error) => {
                    tracing::warn!(code = "M1802", op = %op.id, "scheduled op failed: {error:#}");
                    ("failed", format!("{error:#}"))
                }
            };
            if let Err(error) = repo::mark_scheduled_op(&pool, op.id, status, Some(&result)).await {
                tracing::warn!(code = "M1801", "could not mark scheduled op: {error:#}");
            }
        }
    }
}

async fn push_if_connected(registry: &Arc<Registry>, node_id: Uuid) {
    if registry.is_connected(node_id).await {
        if let Err(error) = registry.auto_push(node_id).await {
            tracing::warn!(code = "M0406", node = %node_id, %error, "scheduled push failed; reconcile will retry");
        }
    }
}

async fn push_user_nodes(pool: &PgPool, registry: &Arc<Registry>, user_id: Uuid) -> Result<()> {
    for node_id in repo::user_node_ids(pool, user_id).await? {
        push_if_connected(registry, node_id).await;
    }
    Ok(())
}

async fn execute(pool: &PgPool, registry: &Arc<Registry>, op: &ScheduledOp) -> Result<String> {
    let id = op.resource_id;
    match (op.resource_type.as_str(), op.action.as_str()) {
        ("node", "enable") | ("node", "disable") => {
            let on = op.action == "enable";
            if !repo::set_node_enabled(pool, id, on).await? {
                return Err(anyhow!("node not found"));
            }
            push_if_connected(registry, id).await;
            Ok(format!("node {}", if on { "enabled" } else { "disabled" }))
        }
        ("node", "push") => {
            push_if_connected(registry, id).await;
            Ok("node pushed".into())
        }
        ("user", "enable") | ("user", "disable") => {
            let on = op.action == "enable";
            if !repo::set_user_enabled(pool, id, on).await? {
                return Err(anyhow!("user not found"));
            }
            push_user_nodes(pool, registry, id).await?;
            Ok(format!("user {}", if on { "enabled" } else { "disabled" }))
        }
        ("user", "reset-traffic") => {
            if repo::reset_user_traffic(pool, id).await?.is_none() {
                return Err(anyhow!("user not found"));
            }
            push_user_nodes(pool, registry, id).await?;
            Ok("traffic reset".into())
        }
        ("user", "rotate-sub") => {
            if repo::rotate_subscription_token(pool, id).await?.is_none() {
                return Err(anyhow!("user not found"));
            }
            Ok("subscription rotated".into())
        }
        ("inbound", "enable") | ("inbound", "disable") => {
            let on = op.action == "enable";
            match repo::set_inbound_enabled(pool, id, on).await? {
                Some(node_id) => {
                    push_if_connected(registry, node_id).await;
                    Ok(format!(
                        "inbound {}",
                        if on { "enabled" } else { "disabled" }
                    ))
                }
                None => Err(anyhow!("inbound not found")),
            }
        }
        (rt, action) => Err(anyhow!("unsupported action {rt}.{action}")),
    }
}
