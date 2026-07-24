//! assembles a `NodeSpec` (the proto the agent turns into config.json) from db rows:
//! a node's enabled inbounds, each with its enabled users.
use anyhow::Result;
use prost::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sqlx::PgPool;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::db::models::User;
use crate::db::repo;
use crate::pb;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecSummary {
    pub version: u32,
    pub inbounds: Vec<InboundSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundSummary {
    pub tag: String,
    pub core: String,
    pub protocol: String,
    pub listen: String,
    pub port: u32,
    pub network: String,
    pub security: String,
    pub user_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigPreview {
    pub desired_hash: String,
    pub applied_hash: Option<String>,
    pub changed: bool,
    pub baseline_available: bool,
    pub added: Vec<InboundSummary>,
    pub removed: Vec<InboundSummary>,
    pub modified: Vec<InboundSummary>,
    pub restart_cores: Vec<String>,
}

pub fn summarize(spec: &pb::NodeSpec) -> SpecSummary {
    let mut inbounds = spec
        .inbounds
        .iter()
        .map(|inbound| {
            let tls = inbound.tls.as_ref();
            let security = if tls.and_then(|value| value.reality.as_ref()).is_some() {
                "reality"
            } else if tls.is_some_and(|value| value.enabled) {
                "tls"
            } else {
                "none"
            };
            InboundSummary {
                tag: inbound.tag.clone(),
                core: if inbound.core.is_empty() {
                    "singbox".into()
                } else {
                    inbound.core.clone()
                },
                protocol: inbound.r#type.clone(),
                listen: inbound.listen.clone(),
                port: inbound.listen_port,
                network: inbound
                    .transport
                    .as_ref()
                    .map(|value| value.network.as_str())
                    .filter(|value| !value.is_empty())
                    .unwrap_or("tcp")
                    .to_string(),
                security: security.into(),
                user_count: inbound.users.len(),
            }
        })
        .collect::<Vec<_>>();
    inbounds.sort_by(|a, b| a.tag.cmp(&b.tag));
    SpecSummary {
        version: 1,
        inbounds,
    }
}

pub fn preview(
    spec: &pb::NodeSpec,
    applied_hash: Option<String>,
    applied_summary: Option<Json>,
) -> ConfigPreview {
    let desired_hash = crate::auth::spec_hash(&spec.encode_to_vec());
    let desired = summarize(spec);
    let previous =
        applied_summary.and_then(|value| serde_json::from_value::<SpecSummary>(value).ok());
    let baseline_available = previous.is_some();
    let old = previous
        .map(|value| {
            value
                .inbounds
                .into_iter()
                .map(|inbound| (inbound.tag.clone(), inbound))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let new = desired
        .inbounds
        .iter()
        .cloned()
        .map(|inbound| (inbound.tag.clone(), inbound))
        .collect::<BTreeMap<_, _>>();
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();
    let mut restart_cores = BTreeSet::new();

    if baseline_available {
        for (tag, inbound) in &new {
            match old.get(tag) {
                None => {
                    added.push(inbound.clone());
                    restart_cores.insert(inbound.core.clone());
                }
                Some(prior) if prior != inbound => {
                    modified.push(inbound.clone());
                    restart_cores.insert(prior.core.clone());
                    restart_cores.insert(inbound.core.clone());
                }
                Some(_) => {}
            }
        }
        for (tag, inbound) in &old {
            if !new.contains_key(tag) {
                removed.push(inbound.clone());
                restart_cores.insert(inbound.core.clone());
            }
        }
    } else if applied_hash.as_deref() != Some(&desired_hash) {
        added = desired.inbounds.clone();
        restart_cores.extend(added.iter().map(|inbound| inbound.core.clone()));
    }

    ConfigPreview {
        changed: applied_hash.as_deref() != Some(&desired_hash),
        desired_hash,
        applied_hash,
        baseline_available,
        added,
        removed,
        modified,
        restart_cores: restart_cores.into_iter().collect(),
    }
}

pub async fn build_node_spec(pool: &PgPool, node_id: Uuid) -> Result<pb::NodeSpec> {
    let inbound_rows = repo::enabled_node_inbounds(pool, node_id).await?;
    // group-based access: every inbound on this node serves the same users
    // (everyone if the node is ungrouped, else users sharing a group). Fetch once.
    let node_users = repo::users_with_node_access(pool, node_id).await?;
    let mut inbounds = Vec::with_capacity(inbound_rows.len());

    for ib in inbound_rows {
        let listen_port = u32::try_from(ib.listen_port)
            .ok()
            .filter(|port| (1..=65_535).contains(port))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "inbound '{}' has invalid listen_port {}",
                    ib.tag,
                    ib.listen_port
                )
            })?;
        let user_rows = node_users.clone();
        let is_vless = ib.kind == "vless";

        // enforcement: over-quota / expired / disabled users are dropped from the
        // spec, so on the next push their config is removed from the node.
        let mut users: Vec<pb::InboundUser> = user_rows
            .into_iter()
            .filter(User::is_active)
            .map(|u| pb::InboundUser {
                name: u.username,
                uuid: u.uuid.to_string(),
                password: u.password,
                // flow is a vless concept; carried on the inbound, applied per user.
                flow: if is_vless {
                    ib.flow.clone()
                } else {
                    String::new()
                },
                // remaining quota for agent-side local cutoff (0 = unlimited).
                quota_bytes: if u.traffic_limit_bytes > 0 {
                    (u.traffic_limit_bytes - u.used_traffic_bytes).max(0) as u64
                } else {
                    0
                },
            })
            .collect();

        // multihop: if any entry inbounds exit through THIS inbound, add their
        // chain credentials as users so the hop authenticates.
        for (label, chain_uuid, chain_password) in repo::chain_users_for_exit(pool, ib.id).await? {
            users.push(pb::InboundUser {
                name: label,
                uuid: chain_uuid,
                password: chain_password.unwrap_or_default(),
                flow: if is_vless {
                    ib.flow.clone()
                } else {
                    String::new()
                },
                quota_bytes: 0,
            });
        }

        // multihop: if THIS inbound chains to an upstream exit, build the
        // sing-box outbound to it (the agent adds it + a route rule).
        let upstream_outbound_json = match ib.upstream_inbound_id {
            Some(exit_id) => build_chain_outbound(pool, &ib, exit_id).await?,
            None => String::new(),
        };

        let tls = if ib.tls_enabled || ib.shadowtls_handshake_server.is_some() {
            let reality = if ib.reality {
                Some(pb::Reality {
                    private_key: ib.reality_private_key.clone().unwrap_or_default(),
                    short_ids: ib.reality_short_ids.clone(),
                    handshake_server: ib.reality_handshake_server.clone().unwrap_or_default(),
                    handshake_port: match ib.reality_handshake_port {
                        Some(port) => u32::try_from(port)
                            .ok()
                            .filter(|port| (1..=65_535).contains(port))
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "inbound '{}' has invalid reality handshake port {}",
                                    ib.tag,
                                    port
                                )
                            })?,
                        None => 443,
                    },
                })
            } else {
                None
            };
            Some(pb::Tls {
                enabled: ib.tls_enabled,
                server_name: ib.server_name.clone().unwrap_or_default(),
                cert_path: ib.cert_path.clone().unwrap_or_default(),
                key_path: ib.key_path.clone().unwrap_or_default(),
                reality,
                ech: ib.ech,
                utls_fingerprint: ib.utls_fingerprint.clone().unwrap_or_default(),
                shadowtls_handshake_server: ib
                    .shadowtls_handshake_server
                    .clone()
                    .unwrap_or_default(),
                shadowtls_handshake_port: ib
                    .shadowtls_handshake_port
                    .and_then(|port| u32::try_from(port).ok())
                    .unwrap_or(0),
            })
        } else {
            None
        };

        // pass extra through only when it carries something.
        let extra_json = match &ib.extra {
            Json::Null => String::new(),
            Json::Object(m) if m.is_empty() => String::new(),
            v => serde_json::to_string(v)?,
        };

        let transport = if ib.network != "tcp" {
            Some(pb::Transport {
                network: ib.network.clone(),
                path: ib.transport_path.clone().unwrap_or_default(),
                host: ib.transport_host.clone().unwrap_or_default(),
                service_name: ib.transport_service_name.clone().unwrap_or_default(),
                mode: ib.transport_mode.clone().unwrap_or_default(),
            })
        } else {
            None
        };

        inbounds.push(pb::Inbound {
            core: ib.core,
            tag: ib.tag,
            r#type: ib.kind,
            listen: ib.listen,
            listen_port,
            users,
            tls,
            extra_json,
            transport,
            up_mbps: ib.up_mbps.max(0) as u32,
            down_mbps: ib.down_mbps.max(0) as u32,
            upstream_outbound_json,
        });
    }

    // WireGuard / AmneziaWG: a separate data-plane. One peer per active user
    // with access to this node; peers are provisioned lazily here.
    let active_users: Vec<&User> = node_users.iter().filter(|u| u.is_active()).collect();
    let active_ids: std::collections::HashSet<Uuid> = active_users.iter().map(|u| u.id).collect();
    let mut wireguard = Vec::new();
    for iface in repo::enabled_wg_interfaces(pool, node_id).await? {
        let existing = repo::wg_peers_for_interface(pool, iface.id).await?;
        let have: std::collections::HashSet<Uuid> = existing.iter().map(|p| p.user_id).collect();
        for user in &active_users {
            if !have.contains(&user.id) {
                repo::ensure_wg_peer(pool, &iface, user.id).await?;
            }
        }
        let peers = repo::wg_peers_for_interface(pool, iface.id)
            .await?
            .into_iter()
            .filter(|p| active_ids.contains(&p.user_id))
            .map(|p| pb::WireguardPeer {
                public_key: p.public_key,
                allowed_ip: format!("{}/32", p.address),
            })
            .collect();
        let amnezia_params_json = if iface.amnezia {
            serde_json::to_string(&iface.amnezia_params)?
        } else {
            String::new()
        };
        let (_, prefix) = crate::wg::parse_cidr(&iface.address_cidr)?;
        wireguard.push(pb::WireguardInterface {
            name: iface.name,
            listen_port: u32::try_from(iface.listen_port).unwrap_or(0),
            private_key: iface.private_key,
            address: format!(
                "{}/{}",
                crate::wg::server_address(&iface.address_cidr)?,
                prefix
            ),
            mtu: u32::try_from(iface.mtu).unwrap_or(1420),
            amnezia: iface.amnezia,
            amnezia_params_json,
            peers,
        });
    }

    // managed external services (mtproto / naive) run as their own daemons.
    let services = repo::enabled_node_services(pool, node_id)
        .await?
        .into_iter()
        .map(|s| pb::NodeService {
            kind: s.kind,
            name: s.name,
            listen_port: u32::try_from(s.listen_port).unwrap_or(0),
            secret: s.secret.unwrap_or_default(),
            config_json: serde_json::to_string(&s.config).unwrap_or_else(|_| "{}".into()),
        })
        .collect();

    Ok(pb::NodeSpec {
        log_level: "info".into(),
        clash_listen: "127.0.0.1:9090".into(),
        clash_secret: String::new(),
        inbounds,
        wireguard,
        services,
    })
}

/// Build the sing-box outbound JSON that carries an entry inbound's traffic to
/// its exit inbound, tagged `chain-<entry_tag>`. Empty string when chaining is
/// not possible (exit gone, entry not sing-box, or missing chain credential).
async fn build_chain_outbound(
    pool: &PgPool,
    entry: &crate::db::models::Inbound,
    exit_id: Uuid,
) -> Result<String> {
    // the entry outbound lives in the entry node's sing-box config.
    if entry.core == "xray" {
        return Ok(String::new());
    }
    let (Some(uuid), Some(password)) =
        (entry.chain_uuid.as_deref(), entry.chain_password.as_deref())
    else {
        return Ok(String::new());
    };
    let Some(exit) = repo::endpoint_for_inbound(pool, exit_id).await? else {
        return Ok(String::new());
    };
    let mut outbound = crate::subscription::singbox_outbound(uuid, password, &exit)?;
    outbound["tag"] = serde_json::json!(format!("chain-{}", entry.tag));
    Ok(serde_json::to_string(&outbound)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(tag: &str, core: &str, port: u32) -> pb::Inbound {
        pb::Inbound {
            tag: tag.into(),
            core: core.into(),
            r#type: "vless".into(),
            listen: "::".into(),
            listen_port: port,
            ..Default::default()
        }
    }

    #[test]
    fn preview_reports_only_sanitized_structural_changes() {
        let old_spec = pb::NodeSpec {
            inbounds: vec![
                inbound("old", "xray", 443),
                inbound("same", "singbox", 8443),
            ],
            ..Default::default()
        };
        let new_spec = pb::NodeSpec {
            inbounds: vec![
                inbound("new", "xray", 2053),
                inbound("same", "singbox", 9443),
            ],
            ..Default::default()
        };
        let old_summary = serde_json::to_value(summarize(&old_spec)).unwrap();
        let result = preview(&new_spec, Some("old-hash".into()), Some(old_summary));
        assert_eq!(
            result
                .added
                .iter()
                .map(|v| v.tag.as_str())
                .collect::<Vec<_>>(),
            ["new"]
        );
        assert_eq!(
            result
                .removed
                .iter()
                .map(|v| v.tag.as_str())
                .collect::<Vec<_>>(),
            ["old"]
        );
        assert_eq!(
            result
                .modified
                .iter()
                .map(|v| v.tag.as_str())
                .collect::<Vec<_>>(),
            ["same"]
        );
        assert_eq!(result.restart_cores, ["singbox", "xray"]);
    }
}
