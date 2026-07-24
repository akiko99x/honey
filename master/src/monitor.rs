//! Background monitors for the analytics/ops surface:
//!   * `anomaly_loop` — hourly anti-abuse scan for users whose latest completed
//!     hour of traffic dwarfs their own recent baseline; each hit becomes a
//!     deduped in-app + channel alert (event `traffic_anomaly`, code M1610).
//!   * `status_sample_loop` — low-frequency online/offline probe per node from
//!     `last_seen` freshness, feeding the public status page's uptime %; prunes
//!     the sample table to a rolling window.
//!
//! Both are runtime-tunable through `app_settings` (re-read each tick) and safe
//! to run with an empty settings table — code defaults apply.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::repo;
use crate::registry::Registry;

const STATUS_KEEP_DAYS: i64 = 7;

/// Scan for traffic anomalies once per `interval` (nominally hourly). Disabled,
/// factor, floor and baseline width are all live-editable settings.
pub async fn anomaly_loop(pool: PgPool, interval: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(code = "M1610", "anomaly loop up (traffic anti-abuse)");
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        if let Err(error) = anomaly_tick(&pool).await {
            tracing::warn!(code = "M1610", "anomaly scan tripped: {error:#}");
        }
    }
}

async fn anomaly_tick(pool: &PgPool) -> Result<()> {
    if repo::setting_i64(pool, "anomaly_enabled", 1).await == 0 {
        return Ok(());
    }
    let factor_pct = repo::setting_i64(pool, "anomaly_factor_pct", 500)
        .await
        .clamp(150, 100_000);
    let min_mib = repo::setting_i64(pool, "anomaly_min_mib", 5120)
        .await
        .max(0);
    let baseline_hours = repo::setting_i64(pool, "anomaly_baseline_hours", 72)
        .await
        .clamp(6, 720);
    let min_history = repo::setting_i64(pool, "anomaly_min_history_hours", 6)
        .await
        .clamp(1, 240);
    let min_bytes = min_mib.saturating_mul(1024 * 1024);

    let anomalies =
        repo::detect_traffic_anomalies(pool, factor_pct, min_bytes, baseline_hours, min_history)
            .await?;
    // one bucket key per user per hour so a sustained spike alerts at most once
    // an hour even across restarts (dedupe also guards the 30-min cooldown).
    let hour = chrono::Utc::now().format("%Y%m%d%H");
    for a in anomalies {
        let ratio = if a.baseline_bytes > 0 {
            a.last_bytes as f64 / a.baseline_bytes as f64
        } else {
            0.0
        };
        let title = format!("📈 honey: traffic spike — {}", a.username);
        let body = format!(
            "{} used {} in the last hour ({:.1}× its {} baseline of {})",
            a.username,
            human_bytes(a.last_bytes),
            ratio,
            human_hours(baseline_hours),
            human_bytes(a.baseline_bytes),
        );
        crate::notify::alert(
            pool,
            "traffic_anomaly",
            &format!("traffic_anomaly:{}:{}", a.user_id, hour),
            &title,
            &body,
            &a.user_id.to_string(),
        )
        .await;
    }
    Ok(())
}

/// Sample every enabled node's online state each `interval` and keep the table
/// bounded. Runs independently of reconcile so uptime resolution doesn't depend
/// on the (possibly slow) push cadence.
pub async fn status_sample_loop(pool: PgPool, interval: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(code = "M0113", "status uptime sampler up");
    let mut ticks: u64 = 0;
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        if let Err(error) = repo::sample_node_status(&pool).await {
            tracing::warn!(code = "M0113", "status sample failed: {error:#}");
        }
        // prune roughly hourly regardless of sample cadence.
        ticks = ticks.wrapping_add(1);
        if ticks % 60 == 0 {
            if let Err(error) = repo::prune_node_status_samples(&pool, STATUS_KEEP_DAYS).await {
                tracing::warn!(code = "M0113", "status sample prune failed: {error:#}");
            }
        }
    }
}

/// Observe/enforce per-user device limits (anti-sharing) once per `interval`.
/// A "device" is a distinct source IP on the live Clash snapshot. Users over
/// their limit always raise a deduped `device_limit` alert; when the
/// `device_limit_enforce` setting is on, the newest excess connections are
/// closed on their node.
pub async fn device_limit_loop(
    pool: PgPool,
    registry: Arc<Registry>,
    interval: Duration,
) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(code = "M1611", "device-limit monitor up (anti-sharing)");
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        if let Err(error) = device_tick(&pool, &registry).await {
            tracing::warn!(code = "M1611", "device-limit scan tripped: {error:#}");
        }
    }
}

struct ConnRef {
    node_id: Uuid,
    id: String,
    ip: String,
    started_at: i64,
}

async fn device_tick(pool: &PgPool, registry: &Arc<Registry>) -> Result<()> {
    let limited = repo::device_limited_users(pool).await?;
    if limited.is_empty() {
        return Ok(());
    }
    let limits: HashMap<String, (Uuid, i32)> = limited
        .into_iter()
        .map(|(id, name, lim)| (name, (id, lim)))
        .collect();
    let enforce = repo::setting_i64(pool, "device_limit_enforce", 0).await == 1;

    // gather connections for limited users across every connected node.
    let mut per_user: HashMap<String, Vec<ConnRef>> = HashMap::new();
    for node_id in registry.connected_ids().await {
        let conns = match registry
            .connections(node_id, crate::pb::CoreKind::Singbox)
            .await
        {
            Ok(conns) => conns,
            Err(_) => continue,
        };
        for c in conns {
            if c.user.is_empty() || c.source_ip.is_empty() || !limits.contains_key(&c.user) {
                continue;
            }
            per_user.entry(c.user.clone()).or_default().push(ConnRef {
                node_id,
                id: c.id,
                ip: c.source_ip,
                started_at: c.started_at,
            });
        }
    }

    let hour = chrono::Utc::now().format("%Y%m%d%H");
    for (user, conns) in per_user {
        let (user_id, raw_limit) = limits[&user];
        let limit = raw_limit.max(0) as usize;
        if limit == 0 {
            continue;
        }
        // each distinct IP with its earliest session start (unknown -> newest).
        let mut ip_first: HashMap<String, i64> = HashMap::new();
        for c in &conns {
            let started = if c.started_at > 0 {
                c.started_at
            } else {
                i64::MAX
            };
            ip_first
                .entry(c.ip.clone())
                .and_modify(|e| {
                    if started < *e {
                        *e = started;
                    }
                })
                .or_insert(started);
        }
        if ip_first.len() <= limit {
            continue;
        }

        // keep the `limit` oldest devices, cut the newest.
        let mut ranked: Vec<(String, i64)> = ip_first.into_iter().collect();
        ranked.sort_by_key(|(_, first)| *first);
        let cut: std::collections::HashSet<String> =
            ranked.into_iter().skip(limit).map(|(ip, _)| ip).collect();
        let distinct = limit + cut.len();

        let title = format!("👥 honey: device limit exceeded — {user}");
        let body = format!(
            "{user} is connected from {distinct} devices (limit {limit}){}",
            if enforce { "; closing the newest" } else { "" }
        );
        crate::notify::alert(
            pool,
            "device_limit",
            &format!("device_limit:{user_id}:{hour}"),
            &title,
            &body,
            &user_id.to_string(),
        )
        .await;

        if enforce {
            let mut by_node: HashMap<Uuid, Vec<String>> = HashMap::new();
            for c in conns {
                if cut.contains(&c.ip) {
                    by_node.entry(c.node_id).or_default().push(c.id);
                }
            }
            for (node_id, ids) in by_node {
                let n = ids.len();
                match registry
                    .close_connections(node_id, crate::pb::CoreKind::Singbox, ids)
                    .await
                {
                    Ok(closed) => tracing::info!(
                        code = "M1611",
                        user = %user,
                        "device-limit: closed {closed}/{n} excess connections"
                    ),
                    Err(error) => {
                        tracing::warn!(code = "M1611", "device-limit close failed: {error:#}")
                    }
                }
            }
        }
    }
    Ok(())
}

/// Periodically ask each connected node whether its running config matches the
/// applied spec. Skips nodes with a pending push (desired changed but not yet
/// pushed) — only real drift (edited/half-applied running config) alerts.
pub async fn drift_loop(pool: PgPool, registry: Arc<Registry>, interval: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(code = "M1612", "config-drift monitor up");
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        if let Err(error) = drift_tick(&pool, &registry).await {
            tracing::warn!(code = "M1612", "config-drift scan tripped: {error:#}");
        }
    }
}

async fn drift_tick(pool: &PgPool, registry: &Arc<Registry>) -> Result<()> {
    for node_id in registry.connected_ids().await {
        let Some(node) = repo::get_node(pool, node_id).await? else {
            continue;
        };
        let spec = match crate::spec::build_node_spec(pool, node_id).await {
            Ok(spec) => spec,
            Err(_) => continue,
        };
        let preview = crate::spec::preview(
            &spec,
            node.applied_spec_hash.clone(),
            node.applied_spec_summary.clone(),
        );
        if preview.changed {
            continue; // pending push, not tampering
        }
        let cores = match registry.config_drift(node_id, spec).await {
            Ok(cores) => cores,
            Err(_) => continue,
        };
        let drifted: Vec<&str> = cores
            .iter()
            .filter(|c| c.drifted)
            .map(|c| {
                if c.core == crate::pb::CoreKind::Xray as i32 {
                    "xray"
                } else {
                    "singbox"
                }
            })
            .collect();
        if drifted.is_empty() {
            continue;
        }
        let title = format!("⚠️ honey: config drift on {}", node.name);
        let body = format!(
            "{}: running config differs from the applied spec ({})",
            node.name,
            drifted.join(", ")
        );
        crate::notify::alert(
            pool,
            "config_drift",
            &format!("config_drift:{node_id}"),
            &title,
            &body,
            &node_id.to_string(),
        )
        .await;
    }
    Ok(())
}

fn human_bytes(value: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut amount = value.max(0) as f64;
    let mut unit = 0;
    while amount >= 1024.0 && unit < UNITS.len() - 1 {
        amount /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", amount as u64, UNITS[unit])
    } else {
        format!("{amount:.1} {}", UNITS[unit])
    }
}

fn human_hours(hours: i64) -> String {
    if hours % 24 == 0 {
        format!("{}-day", hours / 24)
    } else {
        format!("{hours}-hour")
    }
}
