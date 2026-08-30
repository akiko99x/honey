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
use std::net::{IpAddr, SocketAddr};

use tokio::net::{lookup_host, TcpStream, UdpSocket};
use uuid::Uuid;

use crate::db::repo;

const QUIC_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const QUIC_MIN_INITIAL_SIZE: usize = 1200;
const QUIC_RESERVED_VERSION: [u8; 4] = [0x0a, 0x0a, 0x0a, 0x0a];

fn is_quic(kind: &str) -> bool {
    matches!(kind, "hysteria2" | "tuic")
}

/// Build a padded QUIC long-header packet with an unsupported reserved version.
/// A reachable QUIC endpoint responds with a Version Negotiation packet without
/// requiring application credentials, which makes this suitable for HY2/TUIC
/// data-plane health checks.
fn quic_probe_packet() -> [u8; QUIC_MIN_INITIAL_SIZE] {
    let mut packet = [0u8; QUIC_MIN_INITIAL_SIZE];
    packet[0] = 0xc0; // long header + fixed bit
    packet[1..5].copy_from_slice(&QUIC_RESERVED_VERSION);

    let id = Uuid::new_v4();
    let id = id.as_bytes();
    packet[5] = 8; // destination connection ID length
    packet[6..14].copy_from_slice(&id[..8]);
    packet[14] = 8; // source connection ID length
    packet[15..23].copy_from_slice(&id[8..]);
    packet
}

fn is_quic_version_negotiation(packet: &[u8]) -> bool {
    packet.len() >= 5 && packet[0] & 0x80 != 0 && packet[1..5] == [0, 0, 0, 0]
}

async fn quic_open_addr(addr: SocketAddr) -> bool {
    let bind_addr = match addr.ip() {
        IpAddr::V4(_) => "0.0.0.0:0",
        IpAddr::V6(_) => "[::]:0",
    };
    let Ok(socket) = UdpSocket::bind(bind_addr).await else {
        return false;
    };
    if socket.connect(addr).await.is_err() {
        return false;
    }
    if socket.send(&quic_probe_packet()).await.is_err() {
        return false;
    }

    let mut response = [0u8; 2048];
    matches!(
        tokio::time::timeout(QUIC_PROBE_TIMEOUT, socket.recv(&mut response)).await,
        Ok(Ok(size)) if is_quic_version_negotiation(&response[..size])
    )
}

/// Verify that a UDP/QUIC endpoint responds from at least one resolved address.
pub async fn quic_open(host: &str, port: u16) -> bool {
    let Ok(addrs) = lookup_host((host, port)).await else {
        return false;
    };
    for addr in addrs {
        if quic_open_addr(addr).await {
            return true;
        }
    }
    false
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

/// Probe one inbound and store the verdict. TCP protocols use a connect probe;
/// HY2/TUIC use credential-free QUIC version negotiation over UDP.
pub async fn check_one(
    pool: &PgPool,
    inbound_id: Uuid,
    host: &str,
    port: i32,
    kind: &str,
) -> Result<Option<bool>> {
    let Ok(port) = u16::try_from(port) else {
        return Ok(None);
    };
    let quic = is_quic(kind);
    let ok = if quic {
        quic_open(host, port).await
    } else {
        tcp_open(host, port).await
    };
    let error = if ok {
        None
    } else if quic {
        Some("quic version negotiation failed")
    } else {
        Some("tcp connect failed")
    };
    repo::set_inbound_reachability(pool, inbound_id, Some(ok), error).await?;
    if !ok {
        tracing::warn!(
            code = "M1501",
            %inbound_id,
            endpoint = %format!("{host}:{port}"),
            probe = if quic { "quic" } else { "tcp" },
            "endpoint unreachable from master"
        );
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
    /// None = not probeable from here (for example a dial-mode control port).
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
        let quic = is_quic(&inbound.kind);
        let ok = match u16::try_from(inbound.listen_port) {
            Ok(port) if quic => Some(quic_open(&node.address, port).await),
            Ok(port) => Some(tcp_open(&node.address, port).await),
            Err(_) => None,
        };
        out.push(PreflightTarget {
            kind: "data".into(),
            label: format!("{} ({})", inbound.tag, inbound.kind),
            target,
            reachable: ok,
            detail: match ok {
                Some(true) if quic => "QUIC version negotiation succeeded".into(),
                Some(true) => "TCP port open".into(),
                Some(false) if quic => "QUIC version negotiation failed".into(),
                Some(false) => "TCP connect failed".into(),
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
    fn builds_padded_quic_probe_with_reserved_version() {
        let packet = quic_probe_packet();
        assert_eq!(packet.len(), QUIC_MIN_INITIAL_SIZE);
        assert_eq!(packet[0] & 0xc0, 0xc0);
        assert_eq!(packet[1..5], QUIC_RESERVED_VERSION);
        assert_eq!(packet[5], 8);
        assert_eq!(packet[14], 8);
    }

    #[test]
    fn recognizes_only_quic_version_negotiation() {
        assert!(is_quic_version_negotiation(&[0x80, 0, 0, 0, 0]));
        assert!(!is_quic_version_negotiation(&[0x40, 0, 0, 0, 0]));
        assert!(!is_quic_version_negotiation(&[0x80, 0, 0, 0, 1]));
        assert!(!is_quic_version_negotiation(&[0x80, 0, 0]));
    }

    #[tokio::test]
    async fn quic_probe_accepts_mock_version_negotiation() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();
        let responder = tokio::spawn(async move {
            let mut probe = [0u8; QUIC_MIN_INITIAL_SIZE];
            let (size, peer) = server.recv_from(&mut probe).await.unwrap();
            assert_eq!(size, QUIC_MIN_INITIAL_SIZE);
            assert_eq!(probe[1..5], QUIC_RESERVED_VERSION);
            server.send_to(&[0x80, 0, 0, 0, 0], peer).await.unwrap();
        });

        assert!(quic_open_addr(addr).await);
        responder.await.unwrap();
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
