//! Database row types and API write inputs.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value as Json;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Node {
    pub id: Uuid,
    pub name: String,
    pub address: String,
    pub tls_server_name: String,
    pub grpc_port: i32,
    pub transport: String,
    #[serde(default)]
    pub extra_addresses: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub enabled: bool,
    // drained from subscriptions but control plane stays up (distinct from enabled).
    #[serde(default)]
    pub maintenance: bool,
    pub last_seen: Option<DateTime<Utc>>,
    pub agent_version: Option<String>,
    pub singbox_version: Option<String>,
    pub desired_spec_hash: Option<String>,
    pub applied_spec_hash: Option<String>,
    pub applied_spec_summary: Option<Json>,
    pub applied_at: Option<DateTime<Utc>>,
    pub last_push_at: Option<DateTime<Utc>>,
    pub last_push_status: Option<String>,
    pub last_push_message: Option<String>,
    // monthly provider cost in minor units (cents); 0 = untracked.
    #[serde(default)]
    pub monthly_cost_cents: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// White-label branding (singleton), applied to panel / sub-page / status.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Branding {
    #[serde(skip)]
    pub id: i16,
    pub brand_name: String,
    pub logo_url: String,
    pub accent_color: String,
    pub support_url: String,
    pub support_text: String,
    pub footer_text: String,
    pub sub_welcome: String,
    pub sub_show_imports: bool,
    pub sub_show_downloads: bool,
    pub sub_show_endpoints: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBranding {
    #[serde(default)]
    pub brand_name: Option<String>,
    #[serde(default)]
    pub logo_url: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub support_url: Option<String>,
    #[serde(default)]
    pub support_text: Option<String>,
    #[serde(default)]
    pub footer_text: Option<String>,
    #[serde(default)]
    pub sub_welcome: Option<String>,
    #[serde(default)]
    pub sub_show_imports: Option<bool>,
    #[serde(default)]
    pub sub_show_downloads: Option<bool>,
    #[serde(default)]
    pub sub_show_endpoints: Option<bool>,
}

/// An operator announcement shown on the public subscription and status pages.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Announcement {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub level: String,
    pub enabled: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewAnnouncement {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "default_level")]
    pub level: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAnnouncement {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn default_level() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Admin {
    pub id: Uuid,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: String,
    pub enabled: bool,
    #[serde(default)]
    pub totp_enabled: bool,
    // reseller allocation caps (0 = unlimited); ignored for non-reseller roles.
    #[serde(default)]
    pub max_users: i32,
    #[serde(default)]
    pub user_traffic_ceiling_bytes: i64,
    // reseller total traffic budget (sum over own users; 0 = unlimited) and the
    // commission % kept for billing/payout reporting.
    #[serde(default)]
    pub traffic_limit_bytes: i64,
    #[serde(default)]
    pub commission_percent: i32,
    // custom RBAC: when set, its permission matrix overrides the rank role.
    #[serde(default)]
    pub custom_role_id: Option<Uuid>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AdminSession {
    pub id: Uuid,
    pub admin_id: Uuid,
    pub username: String,
    pub expires_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub user_agent: Option<String>,
    pub remote_addr: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AdminLoginEvent {
    pub id: i64,
    pub admin_id: Option<Uuid>,
    pub username: String,
    pub outcome: String,
    pub remote_addr: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A named, scoped API key. `role` is the scope (owner/admin/operator/viewer);
/// only the key hash is stored, never the token.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ApiKey {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub created_by: Option<Uuid>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A custom RBAC role: a matrix of domain -> level (0 none, 1 read, 2 write).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct CustomRole {
    pub id: Uuid,
    pub name: String,
    pub permissions: Json,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewCustomRole {
    pub name: String,
    #[serde(default)]
    pub permissions: Json,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCustomRole {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub permissions: Option<Json>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AdminIp {
    pub id: Uuid,
    pub cidr: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

/// An outbound notification channel (webhook / discord / slack / telegram).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NotifyChannel {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing)]
    pub target: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A deduplicated operational alert shown inside the panel. Read state is
/// joined per administrator and never mutates the global event.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SystemNotification {
    pub id: Uuid,
    pub event_type: String,
    pub severity: String,
    pub code: String,
    pub title: String,
    pub body: String,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub occurrence_count: i32,
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SystemNotificationView {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub notification: SystemNotification,
    pub read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct NewNotifyChannel {
    pub name: String,
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub events: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNotifyChannel {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<String>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TelegramChat {
    pub chat_id: i64,
    pub role: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

/// A node group — the access model. A node with no group is universal; a user
/// reaches a grouped node only via a shared group.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NodeGroup {
    pub id: Uuid,
    pub name: String,
    pub is_default: bool,
    pub note: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewNodeGroup {
    pub name: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNodeGroup {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Body for setting a node's or user's group membership (full replace).
#[derive(Debug, Deserialize)]
pub struct GroupIds {
    #[serde(default)]
    pub group_ids: Vec<Uuid>,
}

/// A deferred operation the scheduler runs at `run_at`.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ScheduledOp {
    pub id: Uuid,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub action: String,
    pub run_at: DateTime<Utc>,
    pub status: String,
    pub result: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewScheduledOp {
    pub resource_type: String,
    pub resource_id: Uuid,
    pub action: String,
    pub run_at: DateTime<Utc>,
}

/// An immutable snapshot of an entity for change history / revert.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct EntityVersion {
    pub id: i64,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub snapshot: Json,
    pub actor: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AuditEvent {
    pub id: i64,
    pub actor_admin_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub request_id: Uuid,
    pub remote_addr: Option<String>,
    pub details: Json,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NodePushEvent {
    pub id: i64,
    pub node_id: Uuid,
    pub desired_hash: String,
    pub applied_hash: Option<String>,
    pub source: String,
    pub status: String,
    pub message: Option<String>,
    pub actor_admin_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct EnrollmentToken {
    pub id: Uuid,
    pub node_id: Uuid,
    #[serde(skip_serializing)]
    pub token_hash: Vec<u8>,
    pub created_by: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NodeCertificate {
    pub id: Uuid,
    pub node_id: Uuid,
    pub serial_number: String,
    pub fingerprint_sha256: String,
    pub subject: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub issued_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub replaced_by: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PanelDomain {
    pub id: Uuid,
    pub host: String,
    pub base_path: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A domain you own, registered so inbounds/public endpoints can pick it from a
/// validated list instead of free-typing. `proxied` marks a CDN-fronted domain.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ManagedDomain {
    pub id: Uuid,
    pub host: String,
    pub node_id: Option<Uuid>,
    pub proxied: bool,
    pub notes: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub dns_ok: bool,
    pub resolved_ips: Vec<String>,
    pub reachable_443: bool,
    pub cert_not_after: Option<DateTime<Utc>>,
    pub cert_ok: bool,
    pub check_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Aggregate first-run state. It is derived on every request instead of being
/// persisted, so restoring or editing real resources always fixes the setup
/// checklist without a separate completion flag.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct OnboardingSnapshot {
    pub domain_count: i64,
    pub node_count: i64,
    pub inbound_count: i64,
    pub user_count: i64,
    pub subscription_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct NewManagedDomain {
    pub host: String,
    #[serde(default)]
    pub node_id: Option<Uuid>,
    #[serde(default)]
    pub proxied: bool,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateManagedDomain {
    #[serde(default)]
    pub node_id: Patch<Uuid>,
    #[serde(default)]
    pub proxied: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_true() -> bool {
    true
}

/// A versioned routing profile: high-level toggles each client output turns into
/// its own rules (sing-box `route`, Clash `rules`).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct RoutingProfile {
    pub id: Uuid,
    pub name: String,
    pub version: i32,
    pub block_ads: bool,
    pub direct_private: bool,
    pub direct_geosite: Vec<String>,
    pub direct_geoip: Vec<String>,
    pub final_proxy: bool,
    pub is_default: bool,
    pub notes: String,
    // content-filter / parental + custom domain rules (routing depth).
    #[serde(default)]
    pub block_adult: bool,
    #[serde(default)]
    pub block_gambling: bool,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    #[serde(default)]
    pub direct_domains: Vec<String>,
    #[serde(default)]
    pub proxy_domains: Vec<String>,
    // per-app rules: [{ "geosite": "telegram", "action": "direct|proxy|block" }]
    #[serde(default)]
    pub app_rules: Json,
    // client-side DNS hardening (emitted into the sing-box subscription config).
    #[serde(default)]
    pub dns_doh: String,
    #[serde(default)]
    pub dns_fakeip: bool,
    #[serde(default)]
    pub dns_block_plain: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewRoutingProfile {
    pub name: String,
    #[serde(default)]
    pub block_ads: bool,
    #[serde(default = "default_true")]
    pub direct_private: bool,
    #[serde(default)]
    pub direct_geosite: Vec<String>,
    #[serde(default)]
    pub direct_geoip: Vec<String>,
    #[serde(default = "default_true")]
    pub final_proxy: bool,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub block_adult: bool,
    #[serde(default)]
    pub block_gambling: bool,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    #[serde(default)]
    pub direct_domains: Vec<String>,
    #[serde(default)]
    pub proxy_domains: Vec<String>,
    #[serde(default)]
    pub app_rules: Json,
    #[serde(default)]
    pub dns_doh: String,
    #[serde(default)]
    pub dns_fakeip: bool,
    #[serde(default)]
    pub dns_block_plain: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoutingProfile {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub block_ads: Option<bool>,
    #[serde(default)]
    pub direct_private: Option<bool>,
    #[serde(default)]
    pub direct_geosite: Option<Vec<String>>,
    #[serde(default)]
    pub direct_geoip: Option<Vec<String>>,
    #[serde(default)]
    pub final_proxy: Option<bool>,
    #[serde(default)]
    pub is_default: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub block_adult: Option<bool>,
    #[serde(default)]
    pub block_gambling: Option<bool>,
    #[serde(default)]
    pub blocked_domains: Option<Vec<String>>,
    #[serde(default)]
    pub direct_domains: Option<Vec<String>>,
    #[serde(default)]
    pub proxy_domains: Option<Vec<String>>,
    #[serde(default)]
    pub app_rules: Option<Json>,
    #[serde(default)]
    pub dns_doh: Option<String>,
    #[serde(default)]
    pub dns_fakeip: Option<bool>,
    #[serde(default)]
    pub dns_block_plain: Option<bool>,
}

/// One of a user's named subscription links (multi-sub profiles).
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct UserSubscription {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    #[serde(skip_serializing)]
    pub token_hash: Vec<u8>,
    #[serde(skip_serializing)]
    pub token_enc: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Inbound {
    pub id: Uuid,
    pub node_id: Uuid,
    pub tag: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[sqlx(rename = "type")]
    pub kind: String,
    pub core: String,
    pub listen: String,
    pub listen_port: i32,
    pub flow: String,
    pub tls_enabled: bool,
    pub server_name: Option<String>,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub reality: bool,
    #[serde(skip_serializing)]
    pub reality_private_key: Option<String>,
    pub reality_public_key: Option<String>,
    pub reality_short_ids: Vec<String>,
    pub reality_handshake_server: Option<String>,
    pub reality_handshake_port: Option<i32>,
    // network transport (first-class)
    pub network: String,
    pub transport_path: Option<String>,
    pub transport_host: Option<String>,
    pub transport_service_name: Option<String>,
    pub transport_mode: Option<String>,
    // tls extras
    pub ech: bool,
    pub utls_fingerprint: Option<String>,
    pub shadowtls_handshake_server: Option<String>,
    pub shadowtls_handshake_port: Option<i32>,
    pub extra: Json,
    pub enabled: bool,
    #[serde(default)]
    pub reachable: Option<bool>,
    #[serde(default)]
    pub reach_checked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reach_error: Option<String>,
    // rf-resilience: CDN host to front through when blocked; owned SNIs to rotate.
    #[serde(default)]
    pub fallback_host: Option<String>,
    #[serde(default)]
    pub sni_pool: Vec<String>,
    // proactive CDN rotation: candidate fronting hosts; the monitor points
    // transport_host at the lowest-latency reachable one.
    #[serde(default)]
    pub cdn_pool: Vec<String>,
    // traffic shaping: per-inbound bandwidth caps in Mbps, 0 = unlimited.
    // Applied natively for hysteria2; other cores have no per-inbound limiter.
    #[serde(default)]
    pub up_mbps: i32,
    #[serde(default)]
    pub down_mbps: i32,
    // multihop: the exit inbound this entry chains to (null = egress directly).
    #[serde(default)]
    pub upstream_inbound_id: Option<Uuid>,
    #[serde(default)]
    pub chain_uuid: Option<String>,
    #[serde(default, skip_serializing)]
    pub chain_password: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One reachability verdict for the panel's per-inbound history.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ReachabilityReport {
    pub id: i64,
    pub source: String,
    pub reachable: bool,
    pub latency_ms: Option<i32>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One server-aggregated traffic bucket. History is stored hourly; the API may
/// roll it up to days without exposing raw samples or credentials.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TrafficSeriesPoint {
    pub bucket: DateTime<Utc>,
    pub up_bytes: i64,
    pub down_bytes: i64,
}

/// A user whose most recent completed hour of traffic exceeds a multiple of its
/// own recent baseline — surfaced by the anomaly loop as an anti-abuse alert.
#[derive(Debug, Clone, FromRow)]
pub struct TrafficAnomaly {
    pub user_id: Uuid,
    pub username: String,
    pub last_bytes: i64,
    pub baseline_bytes: i64,
}

/// Per-node availability over a window, derived from status samples.
#[derive(Debug, Clone, FromRow)]
pub struct NodeUptime {
    pub node_id: Uuid,
    pub ratio: f64,
    pub samples: i64,
}

/// A recent availability event for the public status page's incident timeline.
#[derive(Debug, Clone, FromRow)]
pub struct StatusIncident {
    pub title: String,
    pub severity: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub occurrence_count: i32,
}

/// Ranked traffic consumer (user or node) for a bounded analytics period.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TrafficRank {
    pub id: Uuid,
    pub name: String,
    pub up_bytes: i64,
    pub down_bytes: i64,
}

/// Per-core usage. Protocol/transport attribution is deliberately not claimed:
/// current agents report counters per user and core, not per inbound.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct TrafficCoreBreakdown {
    pub core: String,
    pub up_bytes: i64,
    pub down_bytes: i64,
}

/// Current fleet state shown next to historical traffic for operator accounts.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct FleetHealthSummary {
    pub nodes_total: i64,
    pub nodes_online: i64,
    pub failed_pushes: i64,
    pub unreachable_endpoints: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    #[serde(default)]
    pub labels: Vec<String>,
    // vless/vmess credential; stored encrypted at rest (text), plaintext in memory.
    pub uuid: String,
    #[serde(skip_serializing)]
    pub password: String,
    // subscription_token itself is never stored (only its hash); not a field here.
    pub enabled: bool,
    pub traffic_limit_bytes: i64,
    pub used_traffic_bytes: i64,
    pub expires_at: Option<DateTime<Utc>>,
    // anti-sharing: max distinct concurrent source IPs; 0 = unlimited.
    #[serde(default)]
    pub device_limit: i32,
    #[serde(default)]
    pub routing_profile_id: Option<Uuid>,
    #[serde(default = "default_quota_interval")]
    pub quota_interval: String,
    #[serde(default)]
    pub quota_reset_at: Option<DateTime<Utc>>,
    // owning admin/reseller; null = system/owner-owned. resellers see only theirs.
    #[serde(default)]
    pub created_by: Option<Uuid>,
    // optional short alias for the subscription URL (/s/<alias>); null = none.
    #[serde(default)]
    pub subscription_alias: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_quota_interval() -> String {
    "none".to_string()
}

impl User {
    pub fn suppressed_reason(&self) -> Option<&'static str> {
        if !self.enabled {
            return Some("disabled");
        }
        if self.expires_at.is_some_and(|expires| expires <= Utc::now()) {
            return Some("expired");
        }
        if self.traffic_limit_bytes > 0 && self.used_traffic_bytes >= self.traffic_limit_bytes {
            return Some("quota");
        }
        None
    }

    pub fn is_active(&self) -> bool {
        self.suppressed_reason().is_none()
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct SubscriptionEndpoint {
    pub inbound_id: Uuid,
    pub node_name: String,
    pub address: String,
    pub tag: String,
    pub kind: String,
    pub core: String,
    pub listen_port: i32,
    pub flow: String,
    pub tls_enabled: bool,
    pub server_name: Option<String>,
    pub reality: bool,
    pub reality_public_key: Option<String>,
    pub reality_short_ids: Vec<String>,
    pub network: String,
    pub transport_path: Option<String>,
    pub transport_host: Option<String>,
    pub transport_service_name: Option<String>,
    pub transport_mode: Option<String>,
    pub utls_fingerprint: Option<String>,
    pub ech: bool,
    pub extra: Json,
    pub extra_addresses: Vec<String>,
}

// Missing, explicit null, and a concrete value must stay distinct for PATCH.
#[derive(Debug, Clone, Default)]
pub enum Patch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T> Deserialize<'de> for Patch<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewNode {
    pub name: String,
    pub address: String,
    #[serde(default = "default_tls_server_name")]
    pub tls_server_name: String,
    #[serde(default = "default_grpc_port")]
    pub grpc_port: i32,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default)]
    pub monthly_cost_cents: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateNode {
    pub name: Option<String>,
    pub address: Option<String>,
    pub tls_server_name: Option<String>,
    pub grpc_port: Option<i32>,
    pub transport: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub extra_addresses: Option<Vec<String>>,
    #[serde(default)]
    pub maintenance: Option<bool>,
    #[serde(default)]
    pub monthly_cost_cents: Option<i64>,
}

fn default_grpc_port() -> i32 {
    8443
}
fn default_tls_server_name() -> String {
    "honey-agent".into()
}
fn default_transport() -> String {
    "serve".into()
}
fn default_core() -> String {
    "singbox".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewInbound {
    pub node_id: Uuid,
    pub tag: String,
    pub kind: String,
    #[serde(default = "default_core")]
    pub core: String,
    pub listen_port: i32,
    #[serde(default)]
    pub flow: String,
    #[serde(default)]
    pub tls_enabled: bool,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub cert_path: Option<String>,
    #[serde(default)]
    pub key_path: Option<String>,
    #[serde(default)]
    pub reality: bool,
    #[serde(default)]
    pub reality_private_key: Option<String>,
    #[serde(default)]
    pub reality_public_key: Option<String>,
    #[serde(default)]
    pub reality_short_ids: Vec<String>,
    #[serde(default)]
    pub reality_handshake_server: Option<String>,
    #[serde(default)]
    pub reality_handshake_port: Option<i32>,
    #[serde(default = "default_network")]
    pub network: String,
    #[serde(default)]
    pub transport_path: Option<String>,
    #[serde(default)]
    pub transport_host: Option<String>,
    #[serde(default)]
    pub transport_service_name: Option<String>,
    #[serde(default)]
    pub transport_mode: Option<String>,
    #[serde(default)]
    pub ech: bool,
    #[serde(default)]
    pub utls_fingerprint: Option<String>,
    #[serde(default)]
    pub shadowtls_handshake_server: Option<String>,
    #[serde(default)]
    pub shadowtls_handshake_port: Option<i32>,
    #[serde(default = "empty_json")]
    pub extra: Json,
    #[serde(default)]
    pub fallback_host: Option<String>,
    #[serde(default)]
    pub sni_pool: Vec<String>,
    #[serde(default)]
    pub cdn_pool: Vec<String>,
    #[serde(default)]
    pub up_mbps: i32,
    #[serde(default)]
    pub down_mbps: i32,
    #[serde(default)]
    pub upstream_inbound_id: Option<Uuid>,
}

fn default_network() -> String {
    "tcp".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateInbound {
    pub tag: Option<String>,
    pub kind: Option<String>,
    pub core: Option<String>,
    pub listen_port: Option<i32>,
    pub flow: Option<String>,
    pub tls_enabled: Option<bool>,
    #[serde(default)]
    pub server_name: Patch<String>,
    #[serde(default)]
    pub cert_path: Patch<String>,
    #[serde(default)]
    pub key_path: Patch<String>,
    pub reality: Option<bool>,
    #[serde(default)]
    pub reality_private_key: Patch<String>,
    #[serde(default)]
    pub reality_public_key: Patch<String>,
    pub reality_short_ids: Option<Vec<String>>,
    #[serde(default)]
    pub reality_handshake_server: Patch<String>,
    #[serde(default)]
    pub reality_handshake_port: Patch<i32>,
    pub network: Option<String>,
    #[serde(default)]
    pub transport_path: Patch<String>,
    #[serde(default)]
    pub transport_host: Patch<String>,
    #[serde(default)]
    pub transport_service_name: Patch<String>,
    #[serde(default)]
    pub transport_mode: Patch<String>,
    pub ech: Option<bool>,
    #[serde(default)]
    pub utls_fingerprint: Patch<String>,
    #[serde(default)]
    pub shadowtls_handshake_server: Patch<String>,
    #[serde(default)]
    pub shadowtls_handshake_port: Patch<i32>,
    pub extra: Option<Json>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub fallback_host: Patch<String>,
    #[serde(default)]
    pub sni_pool: Option<Vec<String>>,
    #[serde(default)]
    pub cdn_pool: Option<Vec<String>>,
    pub up_mbps: Option<i32>,
    pub down_mbps: Option<i32>,
    #[serde(default)]
    pub upstream_inbound_id: Patch<Uuid>,
}

fn empty_json() -> Json {
    Json::Object(Default::default())
}

// --- managed external services (MTProto / NaiveProxy) -----------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct NodeService {
    pub id: Uuid,
    pub node_id: Uuid,
    pub kind: String,
    pub name: String,
    pub listen_port: i32,
    #[serde(skip_serializing)]
    pub secret: Option<String>,
    pub config: Json,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewNodeService {
    pub node_id: Uuid,
    pub kind: String,
    pub name: String,
    pub listen_port: i32,
    #[serde(default = "empty_json")]
    pub config: Json,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateNodeService {
    pub name: Option<String>,
    pub listen_port: Option<i32>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub config: Option<Json>,
}

// --- WireGuard / AmneziaWG --------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct WgInterface {
    pub id: Uuid,
    pub node_id: Uuid,
    pub name: String,
    pub listen_port: i32,
    #[serde(skip_serializing)]
    pub private_key: String,
    pub public_key: String,
    pub address_cidr: String,
    pub dns: String,
    pub mtu: i32,
    pub amnezia: bool,
    pub amnezia_params: Json,
    pub endpoint_host: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewWgInterface {
    pub node_id: Uuid,
    pub name: String,
    pub listen_port: i32,
    #[serde(default = "default_wg_cidr")]
    pub address_cidr: String,
    #[serde(default = "default_wg_dns")]
    pub dns: String,
    #[serde(default = "default_wg_mtu")]
    pub mtu: i32,
    #[serde(default)]
    pub amnezia: bool,
    #[serde(default)]
    pub endpoint_host: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateWgInterface {
    pub name: Option<String>,
    pub listen_port: Option<i32>,
    pub dns: Option<String>,
    pub mtu: Option<i32>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub endpoint_host: Patch<String>,
}

/// One client peer on a WG interface. Not serialized directly (holds the client
/// private key); the subscription renders a config instead.
#[derive(Debug, Clone, FromRow)]
pub struct WgPeer {
    pub id: Uuid,
    pub interface_id: Uuid,
    pub user_id: Uuid,
    pub private_key: String,
    pub public_key: String,
    pub address: String,
    pub created_at: DateTime<Utc>,
}

fn default_wg_cidr() -> String {
    "10.7.0.0/24".into()
}
fn default_wg_dns() -> String {
    "1.1.1.1".into()
}
fn default_wg_mtu() -> i32 {
    1420
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewUser {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub traffic_limit_bytes: i64,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub device_limit: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUser {
    pub username: Option<String>,
    pub password: Option<String>,
    pub enabled: Option<bool>,
    pub traffic_limit_bytes: Option<i64>,
    #[serde(default)]
    pub expires_at: Patch<DateTime<Utc>>,
    pub device_limit: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RotateCredentials {
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SavedView {
    pub id: Uuid,
    pub admin_id: Uuid,
    pub name: String,
    pub resource: String,
    pub definition: Json,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewSavedView {
    pub name: String,
    pub resource: String,
    #[serde(default = "empty_json")]
    pub definition: Json,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSavedView {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub definition: Option<Json>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetLabels {
    #[serde(default)]
    pub labels: Vec<String>,
}
