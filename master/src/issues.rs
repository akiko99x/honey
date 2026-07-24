//! Operational health cockpit.
//!
//! This module deliberately derives issues from the current database state
//! instead of persisting alerts. Stable IDs make the result easy to de-duplicate
//! in clients, while a refresh immediately clears conditions that were fixed.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::db::models::{ManagedDomain, Node, NodeCertificate};
use crate::db::repo;

const NODE_ONLINE_WINDOW: Duration = Duration::minutes(2);
const CERT_EXPIRY_WINDOW: Duration = Duration::days(14);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Critical,
    Warning,
    Info,
}

impl IssueSeverity {
    fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::Warning => 1,
            Self::Info => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    /// Stable identifier for this entity/condition pair.
    pub id: String,
    pub severity: IssueSeverity,
    /// Existing honey diagnostic code associated with the condition.
    pub code: &'static str,
    pub kind: &'static str,
    pub title: String,
    /// Safe operator-facing copy. Raw upstream/database errors are excluded.
    pub message: String,
    pub entity_type: &'static str,
    pub entity_id: Uuid,
    pub entity_label: String,
    pub labels: Vec<String>,
    pub node_id: Option<Uuid>,
    /// A safe action already supported by the authenticated API/UI.
    pub action: Option<&'static str>,
    pub detected_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IssueCounts {
    pub total: usize,
    pub critical: usize,
    pub warning: usize,
    pub info: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssuesResponse {
    pub generated_at: DateTime<Utc>,
    pub counts: IssueCounts,
    pub issues: Vec<Issue>,
}

impl IssuesResponse {
    fn new(generated_at: DateTime<Utc>, mut issues: Vec<Issue>) -> Self {
        issues.sort_by(|left, right| {
            left.severity
                .rank()
                .cmp(&right.severity.rank())
                .then_with(|| right.detected_at.cmp(&left.detected_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        issues.dedup_by(|left, right| left.id == right.id);
        let counts = IssueCounts {
            total: issues.len(),
            critical: issues
                .iter()
                .filter(|issue| issue.severity == IssueSeverity::Critical)
                .count(),
            warning: issues
                .iter()
                .filter(|issue| issue.severity == IssueSeverity::Warning)
                .count(),
            info: issues
                .iter()
                .filter(|issue| issue.severity == IssueSeverity::Info)
                .count(),
        };
        Self {
            generated_at,
            counts,
            issues,
        }
    }
}

#[derive(Debug, Clone)]
struct NodeHealth {
    id: Uuid,
    name: String,
    labels: Vec<String>,
    enabled: bool,
    last_seen: Option<DateTime<Utc>>,
    last_push_at: Option<DateTime<Utc>>,
    last_push_status: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<&Node> for NodeHealth {
    fn from(node: &Node) -> Self {
        Self {
            id: node.id,
            name: node.name.clone(),
            labels: node.labels.clone(),
            enabled: node.enabled,
            last_seen: node.last_seen,
            last_push_at: node.last_push_at,
            last_push_status: node.last_push_status.clone(),
            created_at: node.created_at,
        }
    }
}

#[derive(Debug, Clone)]
struct InboundHealth {
    id: Uuid,
    node_id: Uuid,
    tag: String,
    labels: Vec<String>,
    enabled: bool,
    reachable: Option<bool>,
    checked_at: Option<DateTime<Utc>>,
}

impl
    From<&(
        Uuid,
        Uuid,
        String,
        Vec<String>,
        bool,
        Option<bool>,
        Option<DateTime<Utc>>,
    )> for InboundHealth
{
    fn from(
        inbound: &(
            Uuid,
            Uuid,
            String,
            Vec<String>,
            bool,
            Option<bool>,
            Option<DateTime<Utc>>,
        ),
    ) -> Self {
        Self {
            id: inbound.0,
            node_id: inbound.1,
            tag: inbound.2.clone(),
            labels: inbound.3.clone(),
            enabled: inbound.4,
            reachable: inbound.5,
            checked_at: inbound.6,
        }
    }
}

#[derive(Debug, Clone)]
struct DomainHealth {
    id: Uuid,
    node_id: Option<Uuid>,
    host: String,
    checked_at: Option<DateTime<Utc>>,
    dns_ok: bool,
    reachable: bool,
    cert_ok: bool,
    cert_not_after: Option<DateTime<Utc>>,
}

impl From<&ManagedDomain> for DomainHealth {
    fn from(domain: &ManagedDomain) -> Self {
        Self {
            id: domain.id,
            node_id: domain.node_id,
            host: domain.host.clone(),
            checked_at: domain.last_checked_at,
            dns_ok: domain.dns_ok,
            reachable: domain.reachable_443,
            cert_ok: domain.cert_ok,
            cert_not_after: domain.cert_not_after,
        }
    }
}

#[derive(Debug, Clone)]
struct UserHealth {
    id: Uuid,
    username: String,
    labels: Vec<String>,
    enabled: bool,
    traffic_limit: i64,
    traffic_used: i64,
    expires_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl
    From<&(
        Uuid,
        String,
        Vec<String>,
        bool,
        i64,
        i64,
        Option<DateTime<Utc>>,
        DateTime<Utc>,
    )> for UserHealth
{
    fn from(
        user: &(
            Uuid,
            String,
            Vec<String>,
            bool,
            i64,
            i64,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
        ),
    ) -> Self {
        Self {
            id: user.0,
            username: user.1.clone(),
            labels: user.2.clone(),
            enabled: user.3,
            traffic_limit: user.4,
            traffic_used: user.5,
            expires_at: user.6,
            updated_at: user.7,
        }
    }
}

#[derive(Debug, Clone)]
struct CertificateHealth {
    node_id: Uuid,
    not_before: DateTime<Utc>,
    not_after: DateTime<Utc>,
    revoked: bool,
    issued_at: DateTime<Utc>,
}

impl From<&NodeCertificate> for CertificateHealth {
    fn from(certificate: &NodeCertificate) -> Self {
        Self {
            node_id: certificate.node_id,
            not_before: certificate.not_before,
            not_after: certificate.not_after,
            revoked: certificate.revoked_at.is_some(),
            issued_at: certificate.issued_at,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct HealthSnapshot {
    nodes: Vec<NodeHealth>,
    inbounds: Vec<InboundHealth>,
    domains: Vec<DomainHealth>,
    users: Vec<UserHealth>,
    certificates: Vec<CertificateHealth>,
    subscription_abuse_count: i64,
    subscription_abuse_last_seen: Option<DateTime<Utc>>,
}

/// Collect a point-in-time, read-only health view. No raw diagnostic error
/// fields are copied into the result.
pub async fn collect(pool: &PgPool) -> Result<IssuesResponse> {
    let (nodes, inbounds, domains, users, certificates, subscription_abuse) = tokio::try_join!(
        repo::list_nodes(pool),
        repo::inbound_health_snapshot(pool),
        repo::list_managed_domains(pool),
        repo::user_health_snapshot(pool),
        repo::list_node_certificates_all(pool),
        repo::subscription_abuse_summary(pool),
    )?;
    let snapshot = HealthSnapshot {
        nodes: nodes.iter().map(NodeHealth::from).collect(),
        inbounds: inbounds.iter().map(InboundHealth::from).collect(),
        domains: domains.iter().map(DomainHealth::from).collect(),
        users: users.iter().map(UserHealth::from).collect(),
        certificates: certificates.iter().map(CertificateHealth::from).collect(),
        subscription_abuse_count: subscription_abuse.0,
        subscription_abuse_last_seen: subscription_abuse.1,
    };
    Ok(build(snapshot, Utc::now()))
}

fn build(snapshot: HealthSnapshot, now: DateTime<Utc>) -> IssuesResponse {
    let mut issues = Vec::new();

    if snapshot.subscription_abuse_count > 0 {
        issues.push(Issue {
            id: "system:subscription-abuse".into(),
            severity: IssueSeverity::Warning,
            code: "M1701",
            kind: "subscription",
            title: "Subscription requests are being rate limited".into(),
            message: format!(
                "The public subscription guard blocked {} request(s) during the last 30 minutes.",
                snapshot.subscription_abuse_count
            ),
            entity_type: "system",
            entity_id: Uuid::nil(),
            entity_label: "subscription guard".into(),
            labels: Vec::new(),
            node_id: None,
            action: None,
            detected_at: snapshot.subscription_abuse_last_seen,
        });
    }
    let node_labels: HashMap<Uuid, Vec<String>> = snapshot
        .nodes
        .iter()
        .map(|node| (node.id, node.labels.clone()))
        .collect();

    let labels_for = |own: &[String], node_id: Option<Uuid>| {
        let mut labels = own.to_vec();
        if let Some(inherited) = node_id.and_then(|id| node_labels.get(&id)) {
            labels.extend(inherited.iter().cloned());
        }
        labels.sort();
        labels.dedup();
        labels
    };

    for node in snapshot.nodes.iter().filter(|node| node.enabled) {
        if node
            .last_seen
            .is_none_or(|seen| seen <= now - NODE_ONLINE_WINDOW)
        {
            issues.push(Issue {
                id: format!("node:{}:offline", node.id),
                severity: IssueSeverity::Critical,
                code: "M0409",
                kind: "node",
                title: "Node is offline".into(),
                message: "The enabled node has not reported within the last two minutes.".into(),
                entity_type: "node",
                entity_id: node.id,
                entity_label: node.name.clone(),
                labels: node.labels.clone(),
                node_id: Some(node.id),
                action: None,
                detected_at: node.last_seen.or(Some(node.created_at)),
            });
        }
        if node.last_push_status.as_deref() == Some("failed") {
            issues.push(Issue {
                id: format!("node:{}:push-failed", node.id),
                severity: IssueSeverity::Critical,
                code: "M0406",
                kind: "push",
                title: "Last configuration push failed".into(),
                message: "The node did not apply its most recent desired configuration.".into(),
                entity_type: "node",
                entity_id: node.id,
                entity_label: node.name.clone(),
                labels: node.labels.clone(),
                node_id: Some(node.id),
                action: Some("retry_push"),
                detected_at: node.last_push_at,
            });
        }
    }

    for inbound in snapshot
        .inbounds
        .iter()
        .filter(|inbound| inbound.enabled && inbound.reachable == Some(false))
    {
        issues.push(Issue {
            id: format!("inbound:{}:unreachable", inbound.id),
            severity: IssueSeverity::Warning,
            code: "M1501",
            kind: "inbound",
            title: "Inbound is unreachable".into(),
            message: "The latest reachability probe could not connect to this enabled inbound."
                .into(),
            entity_type: "inbound",
            entity_id: inbound.id,
            entity_label: inbound.tag.clone(),
            labels: labels_for(&inbound.labels, Some(inbound.node_id)),
            node_id: Some(inbound.node_id),
            action: Some("probe_inbound"),
            detected_at: inbound.checked_at,
        });
    }

    for domain in &snapshot.domains {
        let issue = if domain.checked_at.is_none() {
            Some((
                IssueSeverity::Warning,
                "M1302",
                "Managed domain has not been verified",
                "Run the domain check before relying on this hostname.",
            ))
        } else if !domain.dns_ok {
            Some((
                IssueSeverity::Critical,
                "M1302",
                "Managed domain DNS check failed",
                "The managed hostname did not resolve as expected during the latest check.",
            ))
        } else if !domain.reachable {
            Some((
                IssueSeverity::Warning,
                "M1302",
                "Managed domain is not reachable on 443",
                "DNS resolved, but the latest TLS reachability check did not complete.",
            ))
        } else if !domain.cert_ok {
            Some((
                IssueSeverity::Critical,
                "M1301",
                "Managed domain certificate is invalid",
                "The latest check did not find a currently valid certificate for this hostname.",
            ))
        } else if domain
            .cert_not_after
            .is_some_and(|expiry| expiry <= now + CERT_EXPIRY_WINDOW)
        {
            Some((
                IssueSeverity::Warning,
                "M1301",
                "Managed domain certificate expires soon",
                "The certificate expires within fourteen days.",
            ))
        } else {
            None
        };
        if let Some((severity, code, title, message)) = issue {
            issues.push(Issue {
                id: format!("domain:{}:health", domain.id),
                severity,
                code,
                kind: "domain",
                title: title.into(),
                message: message.into(),
                entity_type: "domain",
                entity_id: domain.id,
                entity_label: domain.host.clone(),
                labels: labels_for(&[], domain.node_id),
                node_id: domain.node_id,
                action: Some("verify_domain"),
                detected_at: domain.checked_at,
            });
        }
    }

    for user in &snapshot.users {
        let issue = if !user.enabled {
            Some((
                IssueSeverity::Info,
                "M0703",
                "User is disabled",
                "This user's subscriptions and access are intentionally disabled.",
            ))
        } else if user.expires_at.is_some_and(|expiry| expiry <= now) {
            Some((
                IssueSeverity::Warning,
                "M0703",
                "User access has expired",
                "This user's subscriptions are suppressed because the expiry time has passed.",
            ))
        } else if user.traffic_limit > 0 && user.traffic_used >= user.traffic_limit {
            Some((
                IssueSeverity::Warning,
                "M0703",
                "User quota has been reached",
                "This user's subscriptions are suppressed until traffic is reset or the quota window rolls over.",
            ))
        } else {
            None
        };
        if let Some((severity, code, title, message)) = issue {
            issues.push(Issue {
                id: format!("user:{}:suppressed", user.id),
                severity,
                code,
                kind: "user",
                title: title.into(),
                message: message.into(),
                entity_type: "user",
                entity_id: user.id,
                entity_label: user.username.clone(),
                labels: user.labels.clone(),
                node_id: None,
                action: None,
                detected_at: Some(user.updated_at),
            });
        }
    }

    for node in snapshot.nodes.iter().filter(|node| node.enabled) {
        let inventory: Vec<_> = snapshot
            .certificates
            .iter()
            .filter(|certificate| certificate.node_id == node.id)
            .collect();
        if inventory.is_empty() {
            // Legacy CA-valid nodes intentionally remain supported until their
            // first enrollment, so absence of inventory is not an issue.
            continue;
        }
        let mut active: Vec<_> = inventory
            .iter()
            .copied()
            .filter(|certificate| {
                !certificate.revoked && certificate.not_before <= now && certificate.not_after > now
            })
            .collect();
        active.sort_by_key(|certificate| certificate.not_after);
        if active.is_empty() {
            let detected_at = inventory
                .iter()
                .map(|certificate| certificate.issued_at)
                .max();
            issues.push(Issue {
                id: format!("node:{}:certificate-unusable", node.id),
                severity: IssueSeverity::Critical,
                code: "M0810",
                kind: "certificate",
                title: "Node has no usable agent certificate".into(),
                message: "All enrolled certificates are revoked, expired, or not yet valid.".into(),
                entity_type: "node",
                entity_id: node.id,
                entity_label: node.name.clone(),
                labels: node.labels.clone(),
                node_id: Some(node.id),
                action: None,
                detected_at,
            });
        } else if active[0].not_after <= now + CERT_EXPIRY_WINDOW {
            issues.push(Issue {
                id: format!("node:{}:certificate-expiring", node.id),
                severity: IssueSeverity::Warning,
                code: "M0810",
                kind: "certificate",
                title: "Agent certificate expires soon".into(),
                message: "The next active agent certificate expires within fourteen days.".into(),
                entity_type: "node",
                entity_id: node.id,
                entity_label: node.name.clone(),
                labels: node.labels.clone(),
                node_id: Some(node.id),
                action: None,
                detected_at: Some(active[0].not_after),
            });
        }
    }

    IssuesResponse::new(now, issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-22T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn node(id: Uuid) -> NodeHealth {
        NodeHealth {
            id,
            name: "warsaw-1".into(),
            labels: vec!["region:pl".into()],
            enabled: true,
            last_seen: Some(now()),
            last_push_at: None,
            last_push_status: Some("applied".into()),
            created_at: now() - Duration::days(1),
        }
    }

    #[test]
    fn disabled_nodes_do_not_create_offline_or_certificate_noise() {
        let id = Uuid::new_v4();
        let mut item = node(id);
        item.enabled = false;
        item.last_seen = None;
        let response = build(
            HealthSnapshot {
                nodes: vec![item],
                certificates: vec![CertificateHealth {
                    node_id: id,
                    not_before: now() - Duration::days(10),
                    not_after: now() - Duration::days(1),
                    revoked: true,
                    issued_at: now() - Duration::days(10),
                }],
                ..Default::default()
            },
            now(),
        );
        assert!(response.issues.is_empty());
    }

    #[test]
    fn domain_conditions_are_deduplicated_to_the_highest_priority_state() {
        let id = Uuid::new_v4();
        let response = build(
            HealthSnapshot {
                domains: vec![DomainHealth {
                    id,
                    node_id: None,
                    host: "edge.example.com".into(),
                    checked_at: Some(now()),
                    dns_ok: false,
                    reachable: false,
                    cert_ok: false,
                    cert_not_after: Some(now() - Duration::days(1)),
                }],
                ..Default::default()
            },
            now(),
        );
        assert_eq!(response.issues.len(), 1);
        assert_eq!(response.issues[0].id, format!("domain:{id}:health"));
        assert_eq!(response.issues[0].severity, IssueSeverity::Critical);
        assert!(response.issues[0].title.contains("DNS"));
    }

    #[test]
    fn a_valid_replacement_hides_historical_revocation() {
        let id = Uuid::new_v4();
        let response = build(
            HealthSnapshot {
                nodes: vec![node(id)],
                certificates: vec![
                    CertificateHealth {
                        node_id: id,
                        not_before: now() - Duration::days(30),
                        not_after: now() + Duration::days(30),
                        revoked: true,
                        issued_at: now() - Duration::days(30),
                    },
                    CertificateHealth {
                        node_id: id,
                        not_before: now() - Duration::days(1),
                        not_after: now() + Duration::days(90),
                        revoked: false,
                        issued_at: now() - Duration::days(1),
                    },
                ],
                ..Default::default()
            },
            now(),
        );
        assert!(response
            .issues
            .iter()
            .all(|issue| issue.kind != "certificate"));
    }

    #[test]
    fn issues_are_sorted_by_severity_and_counted() {
        let node_id = Uuid::new_v4();
        let inbound_id = Uuid::new_v4();
        let mut offline = node(node_id);
        offline.last_seen = Some(now() - Duration::minutes(3));
        let response = build(
            HealthSnapshot {
                nodes: vec![offline],
                inbounds: vec![InboundHealth {
                    id: inbound_id,
                    node_id,
                    tag: "vless".into(),
                    labels: vec!["protocol:vless".into()],
                    enabled: true,
                    reachable: Some(false),
                    checked_at: Some(now()),
                }],
                ..Default::default()
            },
            now(),
        );
        assert_eq!(response.counts.critical, 1);
        assert_eq!(response.counts.warning, 1);
        assert_eq!(response.issues[0].severity, IssueSeverity::Critical);
        assert_eq!(response.issues[1].severity, IssueSeverity::Warning);
        assert_eq!(
            response.issues[1].labels,
            vec!["protocol:vless", "region:pl"]
        );
    }

    #[test]
    fn recent_subscription_abuse_is_visible_without_exposing_identity() {
        let response = build(
            HealthSnapshot {
                subscription_abuse_count: 7,
                subscription_abuse_last_seen: Some(now()),
                ..Default::default()
            },
            now(),
        );
        assert_eq!(response.issues.len(), 1);
        let issue = &response.issues[0];
        assert_eq!(issue.id, "system:subscription-abuse");
        assert_eq!(issue.code, "M1701");
        assert_eq!(issue.severity, IssueSeverity::Warning);
        assert!(issue.message.contains('7'));
        assert!(!issue.message.contains("token"));
        assert!(!issue.message.contains("address"));
    }
}
