//! Data-plane reachability: probe each inbound's public port so the panel can
//! distinguish "agent online" (control plane) from "endpoint reachable" (data
//! plane), and so confirmed-down endpoints drop out of subscriptions.
//!
//! The master probes from its own network (a basic "is the port open" check).
//! Ground truth from the target region comes from an external vantage-point
//! checker POSTing to `/inbounds/:id/reachability`.
use std::time::Duration;

use anyhow::Result;
use sqlx::PgPool;
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::db::repo;

/// UDP/QUIC protocols can't be cheaply TCP-probed; leave them "unknown" for an
/// external checker to report.
fn is_udp(kind: &str) -> bool {
    matches!(kind, "hysteria2" | "tuic")
}

pub async fn tcp_open(host: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(5), TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

/// TCP connect latency in ms, or None when the host is unreachable within 5s.
pub async fn tcp_latency(host: &str, port: u16) -> Option<u32> {
    let started = std::time::Instant::now();
    match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect((host, port))).await {
        Ok(Ok(_)) => Some(started.elapsed().as_millis().min(u32::MAX as u128) as u32),
        _ => None,
    }
}

/// Choose the CDN fronting host to switch to: the lowest-latency reachable
/// candidate, but only when it beats the current host by `margin_pct` (or the
/// current host is missing/unreachable). Returns None to keep the current host.
pub fn pick_best_cdn(
    measured: &[(String, Option<u32>)],
    current: Option<&str>,
    margin_pct: u32,
) -> Option<String> {
    // best reachable candidate.
    let best = measured
        .iter()
        .filter_map(|(host, lat)| lat.map(|l| (host, l)))
        .min_by_key(|(_, lat)| *lat)?;
    let best_host = best.0.clone();
    let best_lat = best.1;

    let current_lat = current.and_then(|c| {
        measured
            .iter()
            .find(|(h, _)| h == c)
            .and_then(|(_, lat)| *lat)
    });
    match current_lat {
        // current unreachable / not measured → switch to the best reachable.
        None => (Some(best_host.as_str()) != current).then_some(best_host),
        Some(cur) => {
            if best_host.as_str() == current.unwrap_or("") {
                return None; // already on the best
            }
            // switch only if meaningfully faster.
            let threshold = cur.saturating_mul(100u32.saturating_sub(margin_pct)) / 100;
            (best_lat <= threshold).then_some(best_host)
        }
    }
}

/// Probe one inbound and store the verdict. Returns the reachable state (or None
/// if the protocol isn't probeable from here).
pub async fn check_one(
    pool: &PgPool,
    inbound_id: Uuid,
    host: &str,
    port: i32,
    kind: &str,
) -> Result<Option<bool>> {
    if is_udp(kind) {
        return Ok(None);
    }
    let Ok(port) = u16::try_from(port) else {
        return Ok(None);
    };
    let ok = tcp_open(host, port).await;
    repo::set_inbound_reachability(
        pool,
        inbound_id,
        Some(ok),
        if ok { None } else { Some("tcp connect failed") },
    )
    .await?;
    if !ok {
        tracing::warn!(code = "M1501", %inbound_id, endpoint = %format!("{host}:{port}"), "endpoint unreachable from master");
    }
    Ok(Some(ok))
}

/// One probed target in a pre-rollout preflight report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PreflightTarget {
    /// "control" (agent gRPC) or "data" (an inbound's public port)
    pub kind: String,
    pub label: String,
    pub target: String,
    /// None = not probeable from here (UDP/QUIC, or a dial-mode node)
    pub reachable: Option<bool>,
    pub detail: String,
}

/// Probe a node's control port and its enabled inbounds' public ports. This is
/// a *signal* before a rollout — an open port proves reachability from the
/// master's network only; it is not a guarantee of a "clean" address (a
/// blocklist/reputation check is a separate concern).
pub async fn preflight(
    pool: &PgPool,
    node: &crate::db::models::Node,
) -> Result<Vec<PreflightTarget>> {
    let mut out = Vec::new();

    // control plane: only meaningful when the master dials the node.
    if node.transport == "dial" {
        out.push(PreflightTarget {
            kind: "control".into(),
            label: "agent (dial mode)".into(),
            target: format!("{}:{}", node.address, node.grpc_port),
            reachable: None,
            detail: "dial-mode node connects out; nothing to probe".into(),
        });
    } else {
        let ok = match u16::try_from(node.grpc_port) {
            Ok(port) => Some(tcp_open(&node.address, port).await),
            Err(_) => None,
        };
        out.push(PreflightTarget {
            kind: "control".into(),
            label: "agent gRPC".into(),
            target: format!("{}:{}", node.address, node.grpc_port),
            reachable: ok,
            detail: match ok {
                Some(true) => "port open".into(),
                Some(false) => "tcp connect failed".into(),
                None => "invalid port".into(),
            },
        });
    }

    // data plane: each enabled inbound's public port.
    for inbound in repo::enabled_node_inbounds(pool, node.id).await? {
        let target = format!("{}:{}", node.address, inbound.listen_port);
        if is_udp(&inbound.kind) {
            out.push(PreflightTarget {
                kind: "data".into(),
                label: format!("{} ({})", inbound.tag, inbound.kind),
                target,
                reachable: None,
                detail: "udp/quic — not probeable from the master".into(),
            });
            continue;
        }
        let ok = match u16::try_from(inbound.listen_port) {
            Ok(port) => Some(tcp_open(&node.address, port).await),
            Err(_) => None,
        };
        out.push(PreflightTarget {
            kind: "data".into(),
            label: format!("{} ({})", inbound.tag, inbound.kind),
            target,
            reachable: ok,
            detail: match ok {
                Some(true) => "port open".into(),
                Some(false) => "tcp connect failed".into(),
                None => "invalid port".into(),
            },
        });
    }
    Ok(out)
}

/// Targets that came back confirmed-unreachable (the gate's blocking set).
pub fn failures(targets: &[PreflightTarget]) -> Vec<&PreflightTarget> {
    targets
        .iter()
        .filter(|t| t.reachable == Some(false))
        .collect()
}

/// Proactive CDN rotation: for every inbound with a CDN pool, measure the TCP
/// connect latency to each candidate (:443) and, when the setting is on, point
/// transport_host at the fastest reachable one (with a margin to avoid flapping).
/// Rotated nodes are re-pushed so the change reaches the data plane.
pub async fn cdn_rotate(pool: &PgPool, registry: &std::sync::Arc<crate::registry::Registry>) {
    if repo::setting_i64(pool, "cdn_rotate_enabled", 0).await == 0 {
        return;
    }
    let margin = repo::setting_i64(pool, "cdn_rotate_margin_pct", 30)
        .await
        .clamp(1, 90) as u32;
    let inbounds = match repo::inbounds_with_cdn_pool(pool).await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(code = "M1502", "cdn rotate scan failed: {e:#}");
            return;
        }
    };
    for (id, node_id, current, cdn_pool) in inbounds {
        let mut measured = Vec::with_capacity(cdn_pool.len());
        for host in cdn_pool.iter().filter(|h| !h.trim().is_empty()) {
            measured.push((host.clone(), tcp_latency(host, 443).await));
        }
        if let Some(best) = pick_best_cdn(&measured, current.as_deref(), margin) {
            match repo::set_inbound_transport_host(pool, id, &best).await {
                Ok(_) => {
                    tracing::info!(
                        code = "M1505",
                        %id,
                        "cdn rotate: fronting host -> {best} (was {})",
                        current.as_deref().unwrap_or("none")
                    );
                    let _ = registry.auto_push(node_id).await;
                }
                Err(e) => tracing::warn!(code = "M1502", %id, "cdn rotate update failed: {e:#}"),
            }
        }
    }
}

pub async fn monitor(
    pool: PgPool,
    registry: std::sync::Arc<crate::registry::Registry>,
    tick: Duration,
) -> Result<()> {
    let mut ticker = tokio::time::interval(tick);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!(
        code = "M1500",
        secs = tick.as_secs(),
        "reachability monitor up"
    );
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        let targets = match repo::inbounds_for_reach(&pool).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(code = "M1502", "reachability scan failed: {e:#}");
                continue;
            }
        };
        for (id, host, port, kind) in targets {
            // external vantage checkers are ground truth from the target region;
            // don't let the master's own probe override a recent fleet verdict.
            if repo::has_recent_vantage_report(&pool, id)
                .await
                .unwrap_or(false)
            {
                continue;
            }
            if let Err(e) = check_one(&pool, id, &host, port, &kind).await {
                tracing::debug!(code = "M1502", %id, "reachability check failed: {e:#}");
            }
        }
        // proactive CDN rotation by measured latency (setting-gated).
        cdn_rotate(&pool, &registry).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_current_when_it_is_best_or_close() {
        let m = vec![
            ("a.cdn".to_string(), Some(40)),
            ("b.cdn".to_string(), Some(38)),
        ];
        // b is 5% faster, below the 30% margin → stay on a.
        assert_eq!(pick_best_cdn(&m, Some("a.cdn"), 30), None);
    }

    #[test]
    fn switches_when_much_faster() {
        let m = vec![
            ("a.cdn".to_string(), Some(200)),
            ("b.cdn".to_string(), Some(50)),
        ];
        assert_eq!(
            pick_best_cdn(&m, Some("a.cdn"), 30).as_deref(),
            Some("b.cdn")
        );
    }

    #[test]
    fn switches_when_current_unreachable() {
        let m = vec![("a.cdn".to_string(), None), ("b.cdn".to_string(), Some(90))];
        assert_eq!(
            pick_best_cdn(&m, Some("a.cdn"), 30).as_deref(),
            Some("b.cdn")
        );
    }

    #[test]
    fn none_when_nothing_reachable() {
        let m = vec![("a.cdn".to_string(), None), ("b.cdn".to_string(), None)];
        assert_eq!(pick_best_cdn(&m, Some("a.cdn"), 30), None);
    }
}
