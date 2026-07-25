//! Runtime SQL for nodes, inbounds, users, group access and subscriptions.
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value as Json;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{
    Admin, AdminIp, AdminLoginEvent, AdminSession, Announcement, ApiKey, AuditEvent, Branding,
    CustomRole, EnrollmentToken, EntityVersion, FleetHealthSummary, Inbound, ManagedDomain,
    NewAnnouncement, NewCustomRole, NewInbound, NewNode, NewNodeGroup, NewNodeService,
    NewNotifyChannel, NewRoutingProfile, NewScheduledOp, NewUser, NewWgInterface, Node,
    NodeCertificate, NodeGroup, NodePushEvent, NodeService, NodeUptime, NotifyChannel,
    OnboardingSnapshot, PanelDomain, Patch, RoutingProfile, SavedView, ScheduledOp, StatusIncident,
    SubscriptionEndpoint, SystemNotification, SystemNotificationView, TelegramChat, TrafficAnomaly,
    TrafficCoreBreakdown, TrafficRank, TrafficSeriesPoint, UpdateAnnouncement, UpdateBranding,
    UpdateCustomRole, UpdateInbound, UpdateManagedDomain, UpdateNode, UpdateNodeGroup,
    UpdateNodeService, UpdateNotifyChannel, UpdateRoutingProfile, UpdateUser, UpdateWgInterface,
    User, UserSubscription, WgInterface, WgPeer,
};
use crate::auth;
use crate::secret;

// secrets (user password, reality private key) are encrypted at rest; decrypt on
// the way out so the rest of the code sees plaintext.
fn decrypt_user(mut user: User) -> Result<User> {
    user.password = secret::decrypt(&user.password)?;
    user.uuid = secret::decrypt(&user.uuid)?;
    Ok(user)
}
fn decrypt_users(users: Vec<User>) -> Result<Vec<User>> {
    users.into_iter().map(decrypt_user).collect()
}
fn decrypt_opt_user(user: Option<User>) -> Result<Option<User>> {
    user.map(decrypt_user).transpose()
}
fn decrypt_inbound(mut inbound: Inbound) -> Result<Inbound> {
    if let Some(key) = inbound.reality_private_key.take() {
        inbound.reality_private_key = Some(secret::decrypt(&key)?);
    }
    if let Some(pw) = inbound.chain_password.take() {
        inbound.chain_password = Some(secret::decrypt(&pw)?);
    }
    Ok(inbound)
}
fn decrypt_inbounds(inbounds: Vec<Inbound>) -> Result<Vec<Inbound>> {
    inbounds.into_iter().map(decrypt_inbound).collect()
}
fn decrypt_opt_inbound(inbound: Option<Inbound>) -> Result<Option<Inbound>> {
    inbound.map(decrypt_inbound).transpose()
}

/// Read the real resources that make a first usable subscription. `creator`
/// scopes the user-facing half for resellers; infrastructure counts remain in
/// the row but are omitted by the API for that role.
pub async fn onboarding_snapshot(
    pool: &PgPool,
    creator: Option<Uuid>,
) -> Result<OnboardingSnapshot> {
    Ok(sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM managed_domains) AS domain_count,
           (SELECT count(*) FROM nodes) AS node_count,
           (SELECT count(*) FROM inbounds) AS inbound_count,
           (SELECT count(*) FROM users
              WHERE $1::uuid IS NULL OR created_by = $1) AS user_count,
           (SELECT count(*) FROM users
              WHERE ($1::uuid IS NULL OR created_by = $1)
                AND subscription_token_enc IS NOT NULL) AS subscription_count",
    )
    .bind(creator)
    .fetch_one(pool)
    .await?)
}

pub async fn list_panel_domains(pool: &PgPool) -> Result<Vec<PanelDomain>> {
    Ok(
        sqlx::query_as("SELECT * FROM panel_domains ORDER BY host, base_path")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn add_panel_domain(pool: &PgPool, host: &str, base_path: &str) -> Result<PanelDomain> {
    Ok(sqlx::query_as(
        "INSERT INTO panel_domains (host, base_path) VALUES ($1, $2)
         ON CONFLICT (host, base_path) DO UPDATE SET enabled = true
         RETURNING *",
    )
    .bind(host)
    .bind(base_path)
    .fetch_one(pool)
    .await?)
}

pub async fn remove_panel_domain(pool: &PgPool, host: &str, base_path: &str) -> Result<bool> {
    Ok(
        sqlx::query("DELETE FROM panel_domains WHERE host = $1 AND base_path = $2")
            .bind(host)
            .bind(base_path)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

pub async fn find_panel_domain(
    pool: &PgPool,
    host: &str,
    request_path: &str,
) -> Result<Option<PanelDomain>> {
    Ok(sqlx::query_as(
        "SELECT * FROM panel_domains
         WHERE enabled AND host = $1
           AND (base_path = $2 OR left($2, char_length(base_path) + 1) = base_path || '/')
         ORDER BY char_length(base_path) DESC LIMIT 1",
    )
    .bind(host)
    .bind(request_path)
    .fetch_optional(pool)
    .await?)
}

// --- managed domains (data-plane owned-domain registry) --------------------

pub async fn list_managed_domains(pool: &PgPool) -> Result<Vec<ManagedDomain>> {
    Ok(
        sqlx::query_as("SELECT * FROM managed_domains ORDER BY host")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn get_managed_domain(pool: &PgPool, id: Uuid) -> Result<Option<ManagedDomain>> {
    Ok(
        sqlx::query_as("SELECT * FROM managed_domains WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn create_managed_domain(
    pool: &PgPool,
    host: &str,
    node_id: Option<Uuid>,
    proxied: bool,
    notes: &str,
) -> Result<ManagedDomain> {
    Ok(sqlx::query_as(
        "INSERT INTO managed_domains (host, node_id, proxied, notes)
         VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(host)
    .bind(node_id)
    .bind(proxied)
    .bind(notes)
    .fetch_one(pool)
    .await?)
}

pub async fn update_managed_domain(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateManagedDomain,
) -> Result<Option<ManagedDomain>> {
    let Some(current) = get_managed_domain(pool, id).await? else {
        return Ok(None);
    };
    let node_id = match input.node_id {
        Patch::Missing => current.node_id,
        Patch::Null => None,
        Patch::Value(v) => Some(v),
    };
    let proxied = input.proxied.unwrap_or(current.proxied);
    let notes = input.notes.clone().unwrap_or(current.notes);
    Ok(sqlx::query_as(
        "UPDATE managed_domains SET node_id = $2, proxied = $3, notes = $4
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(node_id)
    .bind(proxied)
    .bind(notes)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_managed_domain(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM managed_domains WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// record the outcome of a verify run.
#[allow(clippy::too_many_arguments)]
pub async fn set_managed_domain_check(
    pool: &PgPool,
    id: Uuid,
    dns_ok: bool,
    resolved_ips: &[String],
    reachable_443: bool,
    cert_not_after: Option<DateTime<Utc>>,
    cert_ok: bool,
    check_error: Option<&str>,
) -> Result<Option<ManagedDomain>> {
    Ok(sqlx::query_as(
        "UPDATE managed_domains
         SET last_checked_at = now(), dns_ok = $2, resolved_ips = $3,
             reachable_443 = $4, cert_not_after = $5, cert_ok = $6, check_error = $7
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(dns_ok)
    .bind(resolved_ips)
    .bind(reachable_443)
    .bind(cert_not_after)
    .bind(cert_ok)
    .bind(check_error)
    .fetch_optional(pool)
    .await?)
}

// --- routing profiles ------------------------------------------------------

pub async fn list_routing_profiles(pool: &PgPool) -> Result<Vec<RoutingProfile>> {
    Ok(
        sqlx::query_as("SELECT * FROM routing_profiles ORDER BY name")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn get_routing_profile(pool: &PgPool, id: Uuid) -> Result<Option<RoutingProfile>> {
    Ok(
        sqlx::query_as("SELECT * FROM routing_profiles WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_default_routing_profile(pool: &PgPool) -> Result<Option<RoutingProfile>> {
    Ok(
        sqlx::query_as("SELECT * FROM routing_profiles WHERE is_default LIMIT 1")
            .fetch_optional(pool)
            .await?,
    )
}

/// the profile a user gets: their pinned one, else the default (if any).
pub async fn routing_profile_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<RoutingProfile>> {
    let pinned: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT routing_profile_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    match pinned.flatten() {
        Some(id) => get_routing_profile(pool, id).await,
        None => get_default_routing_profile(pool).await,
    }
}

pub async fn create_routing_profile(
    pool: &PgPool,
    input: &NewRoutingProfile,
) -> Result<RoutingProfile> {
    let mut tx = pool.begin().await?;
    if input.is_default {
        sqlx::query("UPDATE routing_profiles SET is_default = false WHERE is_default")
            .execute(&mut *tx)
            .await?;
    }
    let profile = sqlx::query_as(
        "INSERT INTO routing_profiles
           (name, block_ads, direct_private, direct_geosite, direct_geoip, final_proxy, is_default, notes,
            block_adult, block_gambling, blocked_domains, direct_domains, proxy_domains, app_rules,
            dns_doh, dns_fakeip, dns_block_plain)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) RETURNING *",
    )
    .bind(input.name.trim())
    .bind(input.block_ads)
    .bind(input.direct_private)
    .bind(&input.direct_geosite)
    .bind(&input.direct_geoip)
    .bind(input.final_proxy)
    .bind(input.is_default)
    .bind(input.notes.trim())
    .bind(input.block_adult)
    .bind(input.block_gambling)
    .bind(&input.blocked_domains)
    .bind(&input.direct_domains)
    .bind(&input.proxy_domains)
    .bind(&input.app_rules)
    .bind(input.dns_doh.trim())
    .bind(input.dns_fakeip)
    .bind(input.dns_block_plain)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(profile)
}

pub async fn update_routing_profile(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateRoutingProfile,
) -> Result<Option<RoutingProfile>> {
    let Some(cur) = get_routing_profile(pool, id).await? else {
        return Ok(None);
    };
    let name = input.name.clone().unwrap_or(cur.name);
    let block_ads = input.block_ads.unwrap_or(cur.block_ads);
    let direct_private = input.direct_private.unwrap_or(cur.direct_private);
    let direct_geosite = input.direct_geosite.clone().unwrap_or(cur.direct_geosite);
    let direct_geoip = input.direct_geoip.clone().unwrap_or(cur.direct_geoip);
    let final_proxy = input.final_proxy.unwrap_or(cur.final_proxy);
    let is_default = input.is_default.unwrap_or(cur.is_default);
    let notes = input.notes.clone().unwrap_or(cur.notes);
    let block_adult = input.block_adult.unwrap_or(cur.block_adult);
    let block_gambling = input.block_gambling.unwrap_or(cur.block_gambling);
    let blocked_domains = input.blocked_domains.clone().unwrap_or(cur.blocked_domains);
    let direct_domains = input.direct_domains.clone().unwrap_or(cur.direct_domains);
    let proxy_domains = input.proxy_domains.clone().unwrap_or(cur.proxy_domains);
    let app_rules = input.app_rules.clone().unwrap_or(cur.app_rules);
    let dns_doh = input.dns_doh.clone().unwrap_or(cur.dns_doh);
    let dns_fakeip = input.dns_fakeip.unwrap_or(cur.dns_fakeip);
    let dns_block_plain = input.dns_block_plain.unwrap_or(cur.dns_block_plain);

    let mut tx = pool.begin().await?;
    if is_default {
        sqlx::query("UPDATE routing_profiles SET is_default = false WHERE is_default AND id <> $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let profile = sqlx::query_as(
        "UPDATE routing_profiles SET name = $2, block_ads = $3, direct_private = $4,
           direct_geosite = $5, direct_geoip = $6, final_proxy = $7, is_default = $8,
           notes = $9, block_adult = $10, block_gambling = $11, blocked_domains = $12,
           direct_domains = $13, proxy_domains = $14, app_rules = $15,
           dns_doh = $16, dns_fakeip = $17, dns_block_plain = $18, version = version + 1
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(name.trim())
    .bind(block_ads)
    .bind(direct_private)
    .bind(&direct_geosite)
    .bind(&direct_geoip)
    .bind(final_proxy)
    .bind(is_default)
    .bind(notes.trim())
    .bind(block_adult)
    .bind(block_gambling)
    .bind(&blocked_domains)
    .bind(&direct_domains)
    .bind(&proxy_domains)
    .bind(&app_rules)
    .bind(dns_doh.trim())
    .bind(dns_fakeip)
    .bind(dns_block_plain)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(profile)
}

pub async fn delete_routing_profile(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM routing_profiles WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

pub async fn set_user_routing_profile(
    pool: &PgPool,
    user_id: Uuid,
    profile_id: Option<Uuid>,
) -> Result<bool> {
    Ok(
        sqlx::query("UPDATE users SET routing_profile_id = $2 WHERE id = $1")
            .bind(user_id)
            .bind(profile_id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

// --- notification channels -------------------------------------------------

pub async fn list_notify_channels(pool: &PgPool) -> Result<Vec<NotifyChannel>> {
    Ok(
        sqlx::query_as("SELECT * FROM notify_channels ORDER BY name")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn get_notify_channel(pool: &PgPool, id: Uuid) -> Result<Option<NotifyChannel>> {
    Ok(
        sqlx::query_as("SELECT * FROM notify_channels WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// enabled channels subscribed to `event` (empty events = all).
pub async fn channels_for_event(pool: &PgPool, event: &str) -> Result<Vec<NotifyChannel>> {
    Ok(sqlx::query_as(
        "SELECT * FROM notify_channels
         WHERE enabled AND (cardinality(events) = 0 OR $1 = ANY(events))",
    )
    .bind(event)
    .fetch_all(pool)
    .await?)
}

pub async fn create_notify_channel(
    pool: &PgPool,
    input: &NewNotifyChannel,
) -> Result<NotifyChannel> {
    Ok(sqlx::query_as(
        "INSERT INTO notify_channels (name, kind, target, events)
         VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(input.name.trim())
    .bind(input.kind.trim())
    .bind(input.target.trim())
    .bind(&input.events)
    .fetch_one(pool)
    .await?)
}

pub async fn update_notify_channel(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateNotifyChannel,
) -> Result<Option<NotifyChannel>> {
    let Some(cur) = get_notify_channel(pool, id).await? else {
        return Ok(None);
    };
    let name = input.name.clone().unwrap_or(cur.name);
    let target = input.target.clone().unwrap_or(cur.target);
    let events = input.events.clone().unwrap_or(cur.events);
    let enabled = input.enabled.unwrap_or(cur.enabled);
    Ok(sqlx::query_as(
        "UPDATE notify_channels SET name = $2, target = $3, events = $4, enabled = $5
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(name.trim())
    .bind(target.trim())
    .bind(&events)
    .bind(enabled)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_notify_channel(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM notify_channels WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// Persist an operational alert unless the same dedupe key was seen during the
/// cooldown. The advisory transaction lock makes the check/insert atomic even
/// when multiple background loops report the same condition concurrently.
pub async fn record_system_notification(
    pool: &PgPool,
    event_type: &str,
    dedupe_key: &str,
    severity: &str,
    code: &str,
    title: &str,
    body: &str,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
) -> Result<Option<SystemNotification>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(dedupe_key)
        .execute(&mut *tx)
        .await?;

    let recent = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM system_notifications
         WHERE dedupe_key = $1 AND created_at > now() - interval '30 minutes'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(dedupe_key)
    .fetch_optional(&mut *tx)
    .await?;

    let inserted = if let Some(id) = recent {
        sqlx::query(
            "UPDATE system_notifications
             SET occurrence_count = occurrence_count + 1, last_seen_at = now(),
                 title = $2, body = $3
             WHERE id = $1",
        )
        .bind(id)
        .bind(title)
        .bind(body)
        .execute(&mut *tx)
        .await?;
        None
    } else {
        Some(
            sqlx::query_as(
                "INSERT INTO system_notifications
                   (event_type, dedupe_key, severity, code, title, body, resource_type, resource_id)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                 RETURNING *",
            )
            .bind(event_type)
            .bind(dedupe_key)
            .bind(severity)
            .bind(code)
            .bind(title)
            .bind(body)
            .bind(resource_type)
            .bind(resource_id)
            .fetch_one(&mut *tx)
            .await?,
        )
    };

    sqlx::query("DELETE FROM system_notifications WHERE created_at < now() - interval '90 days'")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(inserted)
}

pub async fn list_system_notifications(
    pool: &PgPool,
    admin_id: Option<Uuid>,
    severity: Option<&str>,
    event_type: Option<&str>,
    unread_only: bool,
    limit: i64,
) -> Result<Vec<SystemNotificationView>> {
    Ok(sqlx::query_as(
        "SELECT n.*, r.read_at
         FROM system_notifications n
         LEFT JOIN admin_notification_reads r
           ON r.notification_id = n.id AND r.admin_id = $1
         WHERE ($2::text IS NULL OR n.severity = $2)
           AND ($3::text IS NULL OR n.event_type = $3)
           AND (NOT $4::boolean OR r.read_at IS NULL)
         ORDER BY n.created_at DESC
         LIMIT $5",
    )
    .bind(admin_id)
    .bind(severity)
    .bind(event_type)
    .bind(unread_only)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

pub async fn count_unread_system_notifications(
    pool: &PgPool,
    admin_id: Option<Uuid>,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM system_notifications n
         LEFT JOIN admin_notification_reads r
           ON r.notification_id = n.id AND r.admin_id = $1
         WHERE r.read_at IS NULL",
    )
    .bind(admin_id)
    .fetch_one(pool)
    .await?)
}

pub async fn prune_system_notifications(pool: &PgPool) -> Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM system_notifications WHERE created_at < now() - interval '90 days'",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

/// Return the number of subscription-guard blocks recorded during the current
/// notification deduplication window and the most recent occurrence. The
/// notification body and dedupe key intentionally contain no raw IP, alias or
/// subscription token.
pub async fn subscription_abuse_summary(pool: &PgPool) -> Result<(i64, Option<DateTime<Utc>>)> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(sum(occurrence_count), 0)::bigint, max(last_seen_at)
         FROM system_notifications
         WHERE event_type = 'subscription_abuse'
           AND last_seen_at > now() - interval '30 minutes'",
    )
    .fetch_one(pool)
    .await?)
}

pub async fn mark_system_notification_read(
    pool: &PgPool,
    admin_id: Uuid,
    notification_id: Uuid,
) -> Result<bool> {
    Ok(sqlx::query(
        "INSERT INTO admin_notification_reads (admin_id, notification_id)
         SELECT $1, id FROM system_notifications WHERE id = $2
         ON CONFLICT (admin_id, notification_id) DO UPDATE SET read_at = now()",
    )
    .bind(admin_id)
    .bind(notification_id)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub async fn mark_all_system_notifications_read(pool: &PgPool, admin_id: Uuid) -> Result<u64> {
    Ok(sqlx::query(
        "INSERT INTO admin_notification_reads (admin_id, notification_id)
         SELECT $1, id FROM system_notifications
         ON CONFLICT (admin_id, notification_id) DO NOTHING",
    )
    .bind(admin_id)
    .execute(pool)
    .await?
    .rows_affected())
}

// --- telegram chat allowlist -----------------------------------------------

pub async fn list_telegram_chats(pool: &PgPool) -> Result<Vec<TelegramChat>> {
    Ok(
        sqlx::query_as("SELECT * FROM telegram_chats ORDER BY created_at")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn telegram_chat_role(pool: &PgPool, chat_id: i64) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT role FROM telegram_chats WHERE chat_id = $1")
            .bind(chat_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn add_telegram_chat(
    pool: &PgPool,
    chat_id: i64,
    role: &str,
    note: &str,
) -> Result<TelegramChat> {
    Ok(sqlx::query_as(
        "INSERT INTO telegram_chats (chat_id, role, note) VALUES ($1, $2, $3)
         ON CONFLICT (chat_id) DO UPDATE SET role = $2, note = $3 RETURNING *",
    )
    .bind(chat_id)
    .bind(role)
    .bind(note)
    .fetch_one(pool)
    .await?)
}

pub async fn delete_telegram_chat(pool: &PgPool, chat_id: i64) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM telegram_chats WHERE chat_id = $1")
        .bind(chat_id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// (nodes_total, nodes_online, users_total, users_active, inbounds_total, traffic_total)
pub async fn metrics_snapshot(pool: &PgPool) -> Result<(i64, i64, i64, i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM nodes), \
           (SELECT count(*) FROM nodes WHERE last_seen > now() - interval '2 minutes'), \
           (SELECT count(*) FROM users), \
           (SELECT count(*) FROM users WHERE enabled \
              AND (traffic_limit_bytes = 0 OR used_traffic_bytes < traffic_limit_bytes) \
              AND (expires_at IS NULL OR expires_at > now())), \
           (SELECT count(*) FROM inbounds), \
           (SELECT coalesce(sum(used_traffic_bytes), 0)::bigint FROM users)",
    )
    .fetch_one(pool)
    .await?)
}

// --- 2FA / admin totp ------------------------------------------------------

pub async fn set_admin_totp_secret(pool: &PgPool, id: Uuid, encrypted: &str) -> Result<bool> {
    Ok(
        sqlx::query("UPDATE admins SET totp_secret = $2, totp_enabled = false WHERE id = $1")
            .bind(id)
            .bind(encrypted)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

pub async fn set_admin_totp_enabled(pool: &PgPool, id: Uuid, enabled: bool) -> Result<bool> {
    Ok(
        sqlx::query("UPDATE admins SET totp_enabled = $2 WHERE id = $1")
            .bind(id)
            .bind(enabled)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

pub async fn clear_admin_totp(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(
        sqlx::query("UPDATE admins SET totp_secret = NULL, totp_enabled = false WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

/// the stored (encrypted) TOTP secret for an admin, if any.
pub async fn get_admin_totp_secret(pool: &PgPool, id: Uuid) -> Result<Option<String>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT totp_secret FROM admins WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|r| r.0))
}

pub async fn count_unused_admin_recovery_codes(pool: &PgPool, admin_id: Uuid) -> Result<i64> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM admin_recovery_codes WHERE admin_id = $1 AND used_at IS NULL",
    )
    .bind(admin_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn replace_admin_recovery_codes(
    pool: &PgPool,
    admin_id: Uuid,
    hashes: &[Vec<u8>],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM admin_recovery_codes WHERE admin_id = $1")
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
    for hash in hashes {
        sqlx::query("INSERT INTO admin_recovery_codes (admin_id, code_hash) VALUES ($1, $2)")
            .bind(admin_id)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Atomically consume a code, so a recovery code can never be replayed.
pub async fn consume_admin_recovery_code(
    pool: &PgPool,
    admin_id: Uuid,
    hash: &[u8],
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE admin_recovery_codes
            SET used_at = now()
          WHERE admin_id = $1 AND code_hash = $2 AND used_at IS NULL",
    )
    .bind(admin_id)
    .bind(hash)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

// --- admin IP allowlist ----------------------------------------------------

pub async fn list_admin_ips(pool: &PgPool) -> Result<Vec<AdminIp>> {
    Ok(
        sqlx::query_as("SELECT * FROM admin_ip_allowlist ORDER BY created_at")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn add_admin_ip(pool: &PgPool, cidr: &str, note: &str) -> Result<AdminIp> {
    Ok(
        sqlx::query_as("INSERT INTO admin_ip_allowlist (cidr, note) VALUES ($1, $2) RETURNING *")
            .bind(cidr)
            .bind(note)
            .fetch_one(pool)
            .await?,
    )
}

pub async fn delete_admin_ip(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM admin_ip_allowlist WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

// --- periodic quotas -------------------------------------------------------

pub async fn set_user_quota_interval(
    pool: &PgPool,
    id: Uuid,
    interval: &str,
    reset_at: Option<DateTime<Utc>>,
) -> Result<bool> {
    Ok(
        sqlx::query("UPDATE users SET quota_interval = $2, quota_reset_at = $3 WHERE id = $1")
            .bind(id)
            .bind(interval)
            .bind(reset_at)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

/// users whose rolling quota window has elapsed and need a reset.
pub async fn due_quota_resets(pool: &PgPool) -> Result<Vec<(Uuid, String)>> {
    Ok(sqlx::query_as(
        "SELECT id, quota_interval FROM users
         WHERE quota_interval <> 'none' AND quota_reset_at IS NOT NULL AND quota_reset_at <= now()",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn advance_quota_reset(pool: &PgPool, id: Uuid, next: DateTime<Utc>) -> Result<()> {
    sqlx::query("UPDATE users SET quota_reset_at = $2 WHERE id = $1")
        .bind(id)
        .bind(next)
        .execute(pool)
        .await?;
    Ok(())
}

// --- reachability ----------------------------------------------------------

/// (inbound_id, node_address, listen_port, kind) for every enabled inbound on an
/// enabled node — the probe targets.
pub async fn inbounds_for_reach(pool: &PgPool) -> Result<Vec<(Uuid, String, i32, String)>> {
    Ok(sqlx::query_as(
        "SELECT i.id, n.address, i.listen_port, i.type
         FROM inbounds i JOIN nodes n ON n.id = i.node_id
         WHERE i.enabled AND n.enabled",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn set_inbound_reachability(
    pool: &PgPool,
    id: Uuid,
    reachable: Option<bool>,
    error: Option<&str>,
) -> Result<bool> {
    Ok(sqlx::query(
        "UPDATE inbounds SET reachable = $2, reach_checked_at = now(), reach_error = $3 WHERE id = $1",
    )
    .bind(id)
    .bind(reachable)
    .bind(error)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

// --- rf-resilience: vantage fleet, consensus, SNI rotation -----------------

/// Record one vantage verdict and recompute the inbound's effective reachability
/// from the recent vantage consensus. Returns the newly-effective state so the
/// caller can react (e.g. rotate SNI on a fresh block). false if inbound missing.
pub async fn record_reachability_report(
    pool: &PgPool,
    inbound_id: Uuid,
    source: &str,
    reachable: bool,
    latency_ms: Option<i32>,
    error: Option<&str>,
) -> Result<Option<Option<bool>>> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM inbounds WHERE id = $1")
        .bind(inbound_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Ok(None);
    }
    sqlx::query(
        "INSERT INTO reachability_reports (inbound_id, source, reachable, latency_ms, error)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(inbound_id)
    .bind(source)
    .bind(reachable)
    .bind(latency_ms)
    .bind(error)
    .execute(pool)
    .await?;
    // consensus over the latest report per source in the last 30 minutes: any
    // vantage seeing a block wins (pessimistic → drain fast for failover).
    let verdict: Option<bool> = sqlx::query_scalar(
        "WITH latest AS (
             SELECT DISTINCT ON (source) reachable
             FROM reachability_reports
             WHERE inbound_id = $1 AND created_at > now() - interval '30 minutes'
             ORDER BY source, created_at DESC
         )
         SELECT CASE WHEN count(*) = 0 THEN NULL
                     WHEN bool_and(reachable) THEN true
                     ELSE false END
         FROM latest",
    )
    .bind(inbound_id)
    .fetch_one(pool)
    .await?;
    let err = if verdict == Some(false) {
        Some("blocked from a vantage checker")
    } else {
        None
    };
    set_inbound_reachability(pool, inbound_id, verdict, err).await?;
    Ok(Some(verdict))
}

/// Whether an inbound has any vantage report in the recent window (so the master
/// TCP probe should defer to the fleet instead of overriding it).
pub async fn has_recent_vantage_report(pool: &PgPool, inbound_id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM reachability_reports
         WHERE inbound_id = $1 AND created_at > now() - interval '30 minutes'",
    )
    .bind(inbound_id)
    .fetch_one(pool)
    .await?
        > 0)
}

pub async fn recent_reachability_reports(
    pool: &PgPool,
    inbound_id: Uuid,
    limit: i64,
) -> Result<Vec<crate::db::models::ReachabilityReport>> {
    Ok(sqlx::query_as(
        "SELECT id, source, reachable, latency_ms, error, created_at
         FROM reachability_reports WHERE inbound_id = $1
         ORDER BY created_at DESC LIMIT $2",
    )
    .bind(inbound_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// Rotate an inbound's SNI to the next value in its pool (skipping the current
/// one), clearing the reachable verdict so it is re-probed. Returns the node id
/// and the new SNI so the caller can re-push, or None if no rotation happened.
/// Inbounds with a non-empty CDN pool, for proactive latency-based rotation:
/// (id, node_id, current transport_host, cdn_pool).
pub async fn inbounds_with_cdn_pool(
    pool: &PgPool,
) -> Result<Vec<(Uuid, Uuid, Option<String>, Vec<String>)>> {
    Ok(sqlx::query_as(
        "SELECT id, node_id, transport_host, cdn_pool FROM inbounds
         WHERE enabled AND cardinality(cdn_pool) > 0",
    )
    .fetch_all(pool)
    .await?)
}

/// Point an inbound's fronting host at a new CDN candidate (proactive rotation).
pub async fn set_inbound_transport_host(pool: &PgPool, id: Uuid, host: &str) -> Result<Uuid> {
    let node_id: Uuid = sqlx::query_scalar(
        "UPDATE inbounds SET transport_host = $2 WHERE id = $1 RETURNING node_id",
    )
    .bind(id)
    .bind(host)
    .fetch_one(pool)
    .await?;
    Ok(node_id)
}

pub async fn rotate_inbound_sni(pool: &PgPool, id: Uuid) -> Result<Option<(Uuid, String)>> {
    let row: Option<(Uuid, Option<String>, Vec<String>)> =
        sqlx::query_as("SELECT node_id, server_name, sni_pool FROM inbounds WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let Some((node_id, current, pool_snis)) = row else {
        return Ok(None);
    };
    let candidates: Vec<&String> = pool_snis
        .iter()
        .filter(|s| !s.trim().is_empty() && Some(s.as_str()) != current.as_deref())
        .collect();
    let Some(next) = candidates.first().map(|s| (*s).clone()) else {
        return Ok(None);
    };
    sqlx::query(
        "UPDATE inbounds SET server_name = $2, reachable = NULL, reach_checked_at = now(),
           reach_error = NULL WHERE id = $1",
    )
    .bind(id)
    .bind(&next)
    .execute(pool)
    .await?;
    Ok(Some((node_id, next)))
}

pub async fn set_inbound_labels(
    pool: &PgPool,
    id: Uuid,
    labels: &[String],
) -> Result<Option<Inbound>> {
    decrypt_opt_inbound(
        sqlx::query_as("UPDATE inbounds SET labels = $2 WHERE id = $1 RETURNING *")
            .bind(id)
            .bind(labels)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn count_nodes(pool: &PgPool) -> Result<i64> {
    Ok(sqlx::query_as::<_, (i64,)>("SELECT count(*) FROM nodes")
        .fetch_one(pool)
        .await?
        .0)
}

pub async fn get_node_by_name(pool: &PgPool, name: &str) -> Result<Option<Node>> {
    Ok(sqlx::query_as("SELECT * FROM nodes WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await?)
}

pub async fn get_node(pool: &PgPool, id: Uuid) -> Result<Option<Node>> {
    Ok(sqlx::query_as("SELECT * FROM nodes WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn list_nodes(pool: &PgPool) -> Result<Vec<Node>> {
    Ok(sqlx::query_as("SELECT * FROM nodes ORDER BY created_at")
        .fetch_all(pool)
        .await?)
}

pub async fn create_node(pool: &PgPool, node: NewNode) -> Result<Node> {
    Ok(sqlx::query_as(
        "INSERT INTO nodes (name, address, tls_server_name, grpc_port, transport, monthly_cost_cents)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(node.name)
    .bind(node.address)
    .bind(node.tls_server_name)
    .bind(node.grpc_port)
    .bind(node.transport)
    .bind(node.monthly_cost_cents.max(0))
    .fetch_one(pool)
    .await?)
}

pub async fn update_node(pool: &PgPool, id: Uuid, node: UpdateNode) -> Result<Option<Node>> {
    Ok(sqlx::query_as(
        "UPDATE nodes SET
           name = COALESCE($2, name), address = COALESCE($3, address),
           tls_server_name = COALESCE($4, tls_server_name),
           grpc_port = COALESCE($5, grpc_port), transport = COALESCE($6, transport),
           enabled = COALESCE($7, enabled),
           extra_addresses = COALESCE($8, extra_addresses),
           maintenance = COALESCE($9, maintenance),
           monthly_cost_cents = COALESCE($10, monthly_cost_cents)
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(node.name)
    .bind(node.address)
    .bind(node.tls_server_name)
    .bind(node.grpc_port)
    .bind(node.transport)
    .bind(node.enabled)
    .bind(node.extra_addresses)
    .bind(node.maintenance)
    .bind(node.monthly_cost_cents.map(|v| v.max(0)))
    .fetch_optional(pool)
    .await?)
}

pub async fn set_node_labels(pool: &PgPool, id: Uuid, labels: &[String]) -> Result<Option<Node>> {
    Ok(
        sqlx::query_as("UPDATE nodes SET labels = $2 WHERE id = $1 RETURNING *")
            .bind(id)
            .bind(labels)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn set_node_tls_name(pool: &PgPool, id: Uuid, tls_server_name: &str) -> Result<()> {
    sqlx::query("UPDATE nodes SET tls_server_name = $2 WHERE id = $1")
        .bind(id)
        .bind(tls_server_name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_node(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM nodes WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

pub async fn touch_node(
    pool: &PgPool,
    node_id: Uuid,
    agent_version: &str,
    singbox_version: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE nodes SET last_seen = now(), agent_version = $2, singbox_version = $3
         WHERE id = $1",
    )
    .bind(node_id)
    .bind(agent_version)
    .bind(singbox_version)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_node_seen(pool: &PgPool, node_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE nodes SET last_seen = now() WHERE id = $1")
        .bind(node_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_inbound(pool: &PgPool, id: Uuid) -> Result<Option<Inbound>> {
    decrypt_opt_inbound(
        sqlx::query_as("SELECT * FROM inbounds WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn create_inbound(pool: &PgPool, inbound: NewInbound) -> Result<Inbound> {
    let reality_private_key = inbound
        .reality_private_key
        .as_deref()
        .map(secret::encrypt)
        .transpose()?;
    let (chain_uuid, chain_password) = chain_credential(inbound.upstream_inbound_id)?;
    decrypt_inbound(
        sqlx::query_as(
            "INSERT INTO inbounds
           (node_id, tag, type, core, listen_port, flow,
            tls_enabled, server_name, cert_path, key_path, reality,
            reality_private_key, reality_public_key, reality_short_ids,
            reality_handshake_server, reality_handshake_port,
            network, transport_path, transport_host, transport_service_name, transport_mode,
            ech, utls_fingerprint, shadowtls_handshake_server, shadowtls_handshake_port, extra,
            fallback_host, sni_pool, up_mbps, down_mbps,
            upstream_inbound_id, chain_uuid, chain_password, cdn_pool)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                 $17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34)
         RETURNING *",
        )
        .bind(inbound.node_id)
        .bind(inbound.tag)
        .bind(inbound.kind)
        .bind(inbound.core)
        .bind(inbound.listen_port)
        .bind(inbound.flow)
        .bind(inbound.tls_enabled)
        .bind(inbound.server_name)
        .bind(inbound.cert_path)
        .bind(inbound.key_path)
        .bind(inbound.reality)
        .bind(reality_private_key)
        .bind(inbound.reality_public_key)
        .bind(inbound.reality_short_ids)
        .bind(inbound.reality_handshake_server)
        .bind(inbound.reality_handshake_port)
        .bind(inbound.network)
        .bind(inbound.transport_path)
        .bind(inbound.transport_host)
        .bind(inbound.transport_service_name)
        .bind(inbound.transport_mode)
        .bind(inbound.ech)
        .bind(inbound.utls_fingerprint)
        .bind(inbound.shadowtls_handshake_server)
        .bind(inbound.shadowtls_handshake_port)
        .bind(inbound.extra)
        .bind(inbound.fallback_host)
        .bind(inbound.sni_pool)
        .bind(inbound.up_mbps.max(0))
        .bind(inbound.down_mbps.max(0))
        .bind(inbound.upstream_inbound_id)
        .bind(chain_uuid)
        .bind(chain_password)
        .bind(&inbound.cdn_pool)
        .fetch_one(pool)
        .await?,
    )
}

/// A fresh chain credential (uuid + encrypted password) when an inbound points
/// at an upstream, else (None, None).
fn chain_credential(upstream: Option<Uuid>) -> Result<(Option<String>, Option<String>)> {
    if upstream.is_none() {
        return Ok((None, None));
    }
    let uuid = Uuid::new_v4().to_string();
    let password = secret::encrypt(&Uuid::new_v4().to_string())?;
    Ok((Some(uuid), Some(password)))
}

pub async fn update_inbound(
    pool: &PgPool,
    id: Uuid,
    inbound: UpdateInbound,
) -> Result<Option<Inbound>> {
    let (server_set, server_name) = patch_parts(inbound.server_name);
    let (cert_set, cert_path) = patch_parts(inbound.cert_path);
    let (key_set, key_path) = patch_parts(inbound.key_path);
    let (private_set, private_key) = patch_parts(inbound.reality_private_key);
    let private_key = private_key.as_deref().map(secret::encrypt).transpose()?;
    let (public_set, public_key) = patch_parts(inbound.reality_public_key);
    let (handshake_set, handshake_server) = patch_parts(inbound.reality_handshake_server);
    let (port_set, handshake_port) = patch_parts(inbound.reality_handshake_port);
    let (tp_set, tp_val) = patch_parts(inbound.transport_path);
    let (th_set, th_val) = patch_parts(inbound.transport_host);
    let (tsn_set, tsn_val) = patch_parts(inbound.transport_service_name);
    let (tm_set, tm_val) = patch_parts(inbound.transport_mode);
    let (utls_set, utls_val) = patch_parts(inbound.utls_fingerprint);
    let (sts_srv_set, sts_srv_val) = patch_parts(inbound.shadowtls_handshake_server);
    let (sts_port_set, sts_port_val) = patch_parts(inbound.shadowtls_handshake_port);
    let (fb_set, fb_val) = patch_parts(inbound.fallback_host);
    // changing the upstream regenerates the chain credential (or clears it).
    let (up_set, up_val) = patch_parts(inbound.upstream_inbound_id);
    let (chain_uuid, chain_password) = if up_set {
        chain_credential(up_val)?
    } else {
        (None, None)
    };

    decrypt_opt_inbound(
        sqlx::query_as(
            "UPDATE inbounds SET
           tag = COALESCE($2, tag), type = COALESCE($3, type), core = COALESCE($4, core),
           listen_port = COALESCE($5, listen_port), flow = COALESCE($6, flow),
           tls_enabled = COALESCE($7, tls_enabled),
           server_name = CASE WHEN $8 THEN $9 ELSE server_name END,
           cert_path = CASE WHEN $10 THEN $11 ELSE cert_path END,
           key_path = CASE WHEN $12 THEN $13 ELSE key_path END,
           reality = COALESCE($14, reality),
           reality_private_key = CASE WHEN $15 THEN $16 ELSE reality_private_key END,
           reality_public_key = CASE WHEN $17 THEN $18 ELSE reality_public_key END,
           reality_short_ids = COALESCE($19, reality_short_ids),
           reality_handshake_server = CASE WHEN $20 THEN $21 ELSE reality_handshake_server END,
           reality_handshake_port = CASE WHEN $22 THEN $23 ELSE reality_handshake_port END,
           extra = COALESCE($24, extra), enabled = COALESCE($25, enabled),
           network = COALESCE($26, network),
           transport_path = CASE WHEN $27 THEN $28 ELSE transport_path END,
           transport_host = CASE WHEN $29 THEN $30 ELSE transport_host END,
           transport_service_name = CASE WHEN $31 THEN $32 ELSE transport_service_name END,
           transport_mode = CASE WHEN $33 THEN $34 ELSE transport_mode END,
           ech = COALESCE($35, ech),
           utls_fingerprint = CASE WHEN $36 THEN $37 ELSE utls_fingerprint END,
           shadowtls_handshake_server = CASE WHEN $38 THEN $39 ELSE shadowtls_handshake_server END,
           shadowtls_handshake_port = CASE WHEN $40 THEN $41 ELSE shadowtls_handshake_port END,
           fallback_host = CASE WHEN $42 THEN $43 ELSE fallback_host END,
           sni_pool = COALESCE($44, sni_pool),
           up_mbps = COALESCE($45, up_mbps),
           down_mbps = COALESCE($46, down_mbps),
           upstream_inbound_id = CASE WHEN $47 THEN $48 ELSE upstream_inbound_id END,
           chain_uuid = CASE WHEN $47 THEN $49 ELSE chain_uuid END,
           chain_password = CASE WHEN $47 THEN $50 ELSE chain_password END,
           cdn_pool = COALESCE($51, cdn_pool)
         WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .bind(inbound.tag)
        .bind(inbound.kind)
        .bind(inbound.core)
        .bind(inbound.listen_port)
        .bind(inbound.flow)
        .bind(inbound.tls_enabled)
        .bind(server_set)
        .bind(server_name)
        .bind(cert_set)
        .bind(cert_path)
        .bind(key_set)
        .bind(key_path)
        .bind(inbound.reality)
        .bind(private_set)
        .bind(private_key)
        .bind(public_set)
        .bind(public_key)
        .bind(inbound.reality_short_ids)
        .bind(handshake_set)
        .bind(handshake_server)
        .bind(port_set)
        .bind(handshake_port)
        .bind(inbound.extra)
        .bind(inbound.enabled)
        .bind(inbound.network)
        .bind(tp_set)
        .bind(tp_val)
        .bind(th_set)
        .bind(th_val)
        .bind(tsn_set)
        .bind(tsn_val)
        .bind(tm_set)
        .bind(tm_val)
        .bind(inbound.ech)
        .bind(utls_set)
        .bind(utls_val)
        .bind(sts_srv_set)
        .bind(sts_srv_val)
        .bind(sts_port_set)
        .bind(sts_port_val)
        .bind(fb_set)
        .bind(fb_val)
        .bind(inbound.sni_pool)
        .bind(inbound.up_mbps.map(|v| v.max(0)))
        .bind(inbound.down_mbps.map(|v| v.max(0)))
        .bind(up_set)
        .bind(up_val)
        .bind(chain_uuid)
        .bind(chain_password)
        .bind(inbound.cdn_pool)
        .fetch_optional(pool)
        .await?,
    )
}

pub async fn delete_inbound(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM inbounds WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

pub async fn node_inbounds(pool: &PgPool, node_id: Uuid) -> Result<Vec<Inbound>> {
    decrypt_inbounds(
        sqlx::query_as("SELECT * FROM inbounds WHERE node_id = $1 ORDER BY listen_port")
            .bind(node_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn enabled_node_inbounds(pool: &PgPool, node_id: Uuid) -> Result<Vec<Inbound>> {
    decrypt_inbounds(
        sqlx::query_as(
            "SELECT * FROM inbounds WHERE node_id = $1 AND enabled ORDER BY listen_port",
        )
        .bind(node_id)
        .fetch_all(pool)
        .await?,
    )
}

/// Minimal fleet-wide inbound state for health views. This projection never
/// reads or decrypts REALITY private keys.
pub async fn inbound_health_snapshot(
    pool: &PgPool,
) -> Result<
    Vec<(
        Uuid,
        Uuid,
        String,
        Vec<String>,
        bool,
        Option<bool>,
        Option<DateTime<Utc>>,
    )>,
> {
    Ok(sqlx::query_as(
        "SELECT id, node_id, tag, labels, enabled, reachable, reach_checked_at
         FROM inbounds ORDER BY node_id, listen_port",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn list_users(pool: &PgPool) -> Result<Vec<User>> {
    decrypt_users(
        sqlx::query_as("SELECT * FROM users ORDER BY username")
            .fetch_all(pool)
            .await?,
    )
}

/// Minimal user lifecycle state for health views. This projection never reads
/// or decrypts credentials or subscription material.
pub async fn user_health_snapshot(
    pool: &PgPool,
) -> Result<
    Vec<(
        Uuid,
        String,
        Vec<String>,
        bool,
        i64,
        i64,
        Option<DateTime<Utc>>,
        DateTime<Utc>,
    )>,
> {
    Ok(sqlx::query_as(
        "SELECT id, username, labels, enabled, traffic_limit_bytes, used_traffic_bytes,
                expires_at, updated_at
         FROM users ORDER BY username",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn get_user(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    decrypt_opt_user(
        sqlx::query_as("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_user_by_name(pool: &PgPool, username: &str) -> Result<Option<User>> {
    decrypt_opt_user(
        sqlx::query_as("SELECT * FROM users WHERE username = $1")
            .bind(username)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn get_user_by_subscription_token(pool: &PgPool, token: Uuid) -> Result<Option<User>> {
    let token_hash = auth::token_hash(&token.to_string());
    if let Some(user) = decrypt_opt_user(
        sqlx::query_as("SELECT * FROM users WHERE subscription_token_hash = $1")
            .bind(&token_hash)
            .fetch_optional(pool)
            .await?,
    )? {
        return Ok(Some(user));
    }
    // Multi-sub: resolve the owning user and use the independently named link
    // as the client-facing profile title.
    let named: Option<(Uuid, String)> =
        sqlx::query_as("SELECT user_id, name FROM user_subscriptions WHERE token_hash = $1")
            .bind(&token_hash)
            .fetch_optional(pool)
            .await?;
    let Some((user_id, name)) = named else {
        return Ok(None);
    };
    let mut user = get_user(pool, user_id).await?;
    if let Some(user) = user.as_mut() {
        user.subscription_title = Some(name);
    }
    Ok(user)
}

// --- multi-subscription profiles -------------------------------------------

pub async fn create_user_subscription(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
) -> Result<(UserSubscription, Uuid)> {
    let token = Uuid::new_v4();
    let token_hash = auth::token_hash(&token.to_string());
    let token_enc = secret::encrypt(&token.to_string())?;
    let row = sqlx::query_as(
        "INSERT INTO user_subscriptions (user_id, name, token_hash, token_enc)
         VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(user_id)
    .bind(name.trim())
    .bind(token_hash)
    .bind(token_enc)
    .fetch_one(pool)
    .await?;
    Ok((row, token))
}

pub async fn list_user_subscriptions(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<UserSubscription>> {
    Ok(
        sqlx::query_as("SELECT * FROM user_subscriptions WHERE user_id = $1 ORDER BY created_at")
            .bind(user_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn delete_user_subscription(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<bool> {
    Ok(
        sqlx::query("DELETE FROM user_subscriptions WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

/// Reveal a named subscription's token (decrypt the stored copy).
pub async fn reveal_user_subscription(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<Option<String>>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT token_enc FROM user_subscriptions WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    match row {
        None => Ok(None),
        Some((None,)) => Ok(Some(None)),
        Some((Some(enc),)) => Ok(Some(Some(secret::decrypt(&enc)?))),
    }
}

pub async fn get_user_by_alias(pool: &PgPool, alias: &str) -> Result<Option<User>> {
    decrypt_opt_user(
        sqlx::query_as("SELECT * FROM users WHERE lower(subscription_alias) = lower($1)")
            .bind(alias.trim())
            .fetch_optional(pool)
            .await?,
    )
}

/// Set (or clear, with None) a user's subscription alias. Returns false if the
/// user does not exist; a duplicate alias surfaces as a unique-violation error.
pub async fn set_user_alias(pool: &PgPool, id: Uuid, alias: Option<&str>) -> Result<bool> {
    let rows = sqlx::query("UPDATE users SET subscription_alias = $2 WHERE id = $1")
        .bind(id)
        .bind(alias)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(rows > 0)
}

/// Creates a user, returning the row plus the plaintext subscription token
/// (only its hash is stored, so this is the one chance to capture it).
pub async fn create_user(
    pool: &PgPool,
    user: NewUser,
    created_by: Option<Uuid>,
    grant_default_group: bool,
) -> Result<(User, Uuid)> {
    let uuid = secret::encrypt(&Uuid::new_v4().to_string())?;
    let password = secret::encrypt(&user.password)?;
    let token = Uuid::new_v4();
    let token_hash = auth::token_hash(&token.to_string());
    let token_enc = secret::encrypt(&token.to_string())?;
    let mut tx = pool.begin().await?;
    let row: User = sqlx::query_as(
        "INSERT INTO users
           (username, uuid, password, subscription_token_hash, subscription_token_enc,
            traffic_limit_bytes, expires_at, created_by, device_limit, subscription_title,
            subscription_description)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING *",
    )
    .bind(user.username)
    .bind(uuid)
    .bind(password)
    .bind(token_hash)
    .bind(token_enc)
    .bind(user.traffic_limit_bytes)
    .bind(user.expires_at)
    .bind(created_by)
    .bind(user.device_limit.max(0))
    .bind(user.subscription_title)
    .bind(user.subscription_description)
    .fetch_one(&mut *tx)
    .await?;
    // owner/admin-created users join the default group (their old universal-ish
    // reach). reseller-created users are granted the reseller's own groups by
    // the handler instead, so we skip the default grant for them.
    if grant_default_group {
        sqlx::query(
            "INSERT INTO user_group_access (user_id, group_id)
             SELECT $1, id FROM node_groups WHERE is_default
             ON CONFLICT DO NOTHING",
        )
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok((decrypt_user(row)?, token))
}

pub async fn update_user(pool: &PgPool, id: Uuid, user: UpdateUser) -> Result<Option<User>> {
    let (expires_set, expires_at) = patch_parts(user.expires_at);
    let (title_set, subscription_title) = patch_parts(user.subscription_title);
    let (description_set, subscription_description) = patch_parts(user.subscription_description);
    let password = user.password.as_deref().map(secret::encrypt).transpose()?;
    decrypt_opt_user(
        sqlx::query_as(
            "UPDATE users SET username = COALESCE($2, username), password = COALESCE($3, password),
           enabled = COALESCE($4, enabled),
           traffic_limit_bytes = COALESCE($5, traffic_limit_bytes),
           expires_at = CASE WHEN $6 THEN $7 ELSE expires_at END,
           device_limit = COALESCE($8, device_limit),
           subscription_title = CASE WHEN $9 THEN $10 ELSE subscription_title END
           ,subscription_description = CASE WHEN $11 THEN $12 ELSE subscription_description END
         WHERE id = $1 RETURNING *",
        )
        .bind(id)
        .bind(user.username)
        .bind(password)
        .bind(user.enabled)
        .bind(user.traffic_limit_bytes)
        .bind(expires_set)
        .bind(expires_at)
        .bind(user.device_limit.map(|v| v.max(0)))
        .bind(title_set)
        .bind(subscription_title)
        .bind(description_set)
        .bind(subscription_description)
        .fetch_optional(pool)
        .await?,
    )
}

pub async fn set_user_labels(pool: &PgPool, id: Uuid, labels: &[String]) -> Result<Option<User>> {
    decrypt_opt_user(
        sqlx::query_as("UPDATE users SET labels = $2 WHERE id = $1 RETURNING *")
            .bind(id)
            .bind(labels)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn list_labels(pool: &PgPool, resource: &str) -> Result<Vec<String>> {
    let sql = match resource {
        "nodes" => "SELECT DISTINCT unnest(labels) AS label FROM nodes ORDER BY label",
        "inbounds" => "SELECT DISTINCT unnest(labels) AS label FROM inbounds ORDER BY label",
        "users" => "SELECT DISTINCT unnest(labels) AS label FROM users ORDER BY label",
        _ => anyhow::bail!("unsupported label resource"),
    };
    Ok(sqlx::query_as::<_, (String,)>(sql)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| row.0)
        .collect())
}

pub async fn list_user_labels_for_creator(pool: &PgPool, admin_id: Uuid) -> Result<Vec<String>> {
    Ok(sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT unnest(labels) AS label
         FROM users WHERE created_by = $1 ORDER BY label",
    )
    .bind(admin_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| row.0)
    .collect())
}

// --- personal saved views -------------------------------------------------

pub async fn list_saved_views(pool: &PgPool, admin_id: Uuid) -> Result<Vec<SavedView>> {
    Ok(sqlx::query_as(
        "SELECT * FROM saved_views WHERE admin_id = $1 ORDER BY resource, lower(name)",
    )
    .bind(admin_id)
    .fetch_all(pool)
    .await?)
}

pub async fn create_saved_view(
    pool: &PgPool,
    admin_id: Uuid,
    name: &str,
    resource: &str,
    definition: &Json,
) -> Result<SavedView> {
    Ok(sqlx::query_as(
        "INSERT INTO saved_views (admin_id, name, resource, definition)
         VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(admin_id)
    .bind(name)
    .bind(resource)
    .bind(definition)
    .fetch_one(pool)
    .await?)
}

pub async fn update_saved_view(
    pool: &PgPool,
    id: Uuid,
    admin_id: Uuid,
    name: Option<&str>,
    definition: Option<&Json>,
) -> Result<Option<SavedView>> {
    Ok(sqlx::query_as(
        "UPDATE saved_views SET name = COALESCE($3, name),
             definition = COALESCE($4, definition)
         WHERE id = $1 AND admin_id = $2 RETURNING *",
    )
    .bind(id)
    .bind(admin_id)
    .bind(name)
    .bind(definition)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_saved_view(pool: &PgPool, id: Uuid, admin_id: Uuid) -> Result<bool> {
    Ok(
        sqlx::query("DELETE FROM saved_views WHERE id = $1 AND admin_id = $2")
            .bind(id)
            .bind(admin_id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

pub async fn delete_user(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

pub async fn rotate_credentials(
    pool: &PgPool,
    id: Uuid,
    password: Option<String>,
) -> Result<Option<(Uuid, String)>> {
    // generate in Rust (not SQL) so the stored values can be encrypted; the
    // plaintext is returned once for the admin to hand to the user.
    let plaintext = password.unwrap_or_else(random_secret);
    let stored = secret::encrypt(&plaintext)?;
    let new_uuid = Uuid::new_v4();
    let uuid_enc = secret::encrypt(&new_uuid.to_string())?;
    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE users SET uuid = $2, password = $3
         WHERE id = $1 RETURNING id",
    )
    .bind(id)
    .bind(uuid_enc)
    .bind(stored)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|_| (new_uuid, plaintext)))
}

fn random_secret() -> String {
    // 256 bits of hex from two v4 UUIDs; plenty for a proxy password.
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub async fn rotate_subscription_token(pool: &PgPool, id: Uuid) -> Result<Option<Uuid>> {
    let token = Uuid::new_v4();
    let token_hash = auth::token_hash(&token.to_string());
    let token_enc = secret::encrypt(&token.to_string())?;
    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE users SET subscription_token_hash = $2, subscription_token_enc = $3
         WHERE id = $1 RETURNING id",
    )
    .bind(id)
    .bind(token_hash)
    .bind(token_enc)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|_| token))
}

/// Re-display the current subscription token without rotating it. Outer `None`
/// = user missing; inner `None` = no encrypted copy (a pre-0024 user that has
/// not rotated since — reveal is impossible, only the hash is stored).
pub async fn reveal_subscription_token(pool: &PgPool, id: Uuid) -> Result<Option<Option<String>>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT subscription_token_enc FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    match row {
        None => Ok(None),
        Some((None,)) => Ok(Some(None)),
        Some((Some(enc),)) => Ok(Some(Some(secret::decrypt(&enc)?))),
    }
}

pub async fn reset_user_traffic(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM node_user_traffic WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let user = sqlx::query_as("UPDATE users SET used_traffic_bytes = 0 WHERE id = $1 RETURNING *")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    decrypt_opt_user(user)
}

/// Re-encrypt every stored secret with the current key. Idempotent: legacy
/// plaintext is wrapped, already-encrypted values are re-wrapped. Used to
/// upgrade an existing database after `HONEY_SECRET_KEY` is introduced.
pub async fn reencrypt_secrets(pool: &PgPool) -> Result<(u64, u64)> {
    let mut users = 0u64;
    for (id, uuid, password, token_enc) in
        sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
            "SELECT id, uuid, password, subscription_token_enc FROM users",
        )
        .fetch_all(pool)
        .await?
    {
        let uuid = secret::encrypt(&secret::decrypt(&uuid)?)?;
        let password = secret::encrypt(&secret::decrypt(&password)?)?;
        let token_enc = token_enc
            .map(|t| secret::decrypt(&t).and_then(|p| secret::encrypt(&p)))
            .transpose()?;
        sqlx::query(
            "UPDATE users SET uuid = $2, password = $3, subscription_token_enc = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(uuid)
        .bind(password)
        .bind(token_enc)
        .execute(pool)
        .await?;
        users += 1;
    }

    for (id, token_enc) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, token_enc FROM user_subscriptions WHERE token_enc IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(token_enc) = token_enc else { continue };
        let value = secret::decrypt(&token_enc).and_then(|p| secret::encrypt(&p))?;
        sqlx::query("UPDATE user_subscriptions SET token_enc = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
    }

    for (table, id, key) in wg_private_keys(pool).await? {
        let value = secret::encrypt(&secret::decrypt(&key)?)?;
        sqlx::query(&format!(
            "UPDATE {table} SET private_key = $2 WHERE id = $1"
        ))
        .bind(id)
        .bind(value)
        .execute(pool)
        .await?;
    }

    for (id, sec) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, secret FROM node_services WHERE secret IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(sec) = sec else { continue };
        let value = secret::encrypt(&secret::decrypt(&sec)?)?;
        sqlx::query("UPDATE node_services SET secret = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
    }

    let mut inbounds = 0u64;
    for (id, stored) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, reality_private_key FROM inbounds WHERE reality_private_key IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(stored) = stored else { continue };
        let value = secret::encrypt(&secret::decrypt(&stored)?)?;
        sqlx::query("UPDATE inbounds SET reality_private_key = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
        inbounds += 1;
    }

    for (id, stored) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, chain_password FROM inbounds WHERE chain_password IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(stored) = stored else { continue };
        let value = secret::encrypt(&secret::decrypt(&stored)?)?;
        sqlx::query("UPDATE inbounds SET chain_password = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
    }

    Ok((users, inbounds))
}

/// Rotate every stored secret from the old master key to a new one.
/// Returns (users, inbounds, admins) rows rewritten.
pub async fn rekey_secrets(pool: &PgPool, old: &str, new: &str) -> Result<(u64, u64, u64)> {
    let mut users = 0u64;
    for (id, uuid, password, token_enc) in
        sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
            "SELECT id, uuid, password, subscription_token_enc FROM users",
        )
        .fetch_all(pool)
        .await?
    {
        let uuid = secret::rekey_value(old, new, &uuid)?;
        let password = secret::rekey_value(old, new, &password)?;
        let token_enc = token_enc
            .map(|t| secret::rekey_value(old, new, &t))
            .transpose()?;
        sqlx::query(
            "UPDATE users SET uuid = $2, password = $3, subscription_token_enc = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(uuid)
        .bind(password)
        .bind(token_enc)
        .execute(pool)
        .await?;
        users += 1;
    }

    for (id, token_enc) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, token_enc FROM user_subscriptions WHERE token_enc IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(token_enc) = token_enc else { continue };
        let value = secret::rekey_value(old, new, &token_enc)?;
        sqlx::query("UPDATE user_subscriptions SET token_enc = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
    }

    for (table, id, key) in wg_private_keys(pool).await? {
        let value = secret::rekey_value(old, new, &key)?;
        sqlx::query(&format!(
            "UPDATE {table} SET private_key = $2 WHERE id = $1"
        ))
        .bind(id)
        .bind(value)
        .execute(pool)
        .await?;
    }

    for (id, sec) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, secret FROM node_services WHERE secret IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(sec) = sec else { continue };
        let value = secret::rekey_value(old, new, &sec)?;
        sqlx::query("UPDATE node_services SET secret = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
    }

    let mut inbounds = 0u64;
    for (id, stored) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, reality_private_key FROM inbounds WHERE reality_private_key IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(stored) = stored else { continue };
        let value = secret::rekey_value(old, new, &stored)?;
        sqlx::query("UPDATE inbounds SET reality_private_key = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
        inbounds += 1;
    }

    for (id, stored) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, chain_password FROM inbounds WHERE chain_password IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(stored) = stored else { continue };
        let value = secret::rekey_value(old, new, &stored)?;
        sqlx::query("UPDATE inbounds SET chain_password = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
    }

    let mut admins = 0u64;
    for (id, stored) in sqlx::query_as::<_, (Uuid, Option<String>)>(
        "SELECT id, totp_secret FROM admins WHERE totp_secret IS NOT NULL",
    )
    .fetch_all(pool)
    .await?
    {
        let Some(stored) = stored else { continue };
        let value = secret::rekey_value(old, new, &stored)?;
        sqlx::query("UPDATE admins SET totp_secret = $2 WHERE id = $1")
            .bind(id)
            .bind(value)
            .execute(pool)
            .await?;
        admins += 1;
    }

    Ok((users, inbounds, admins))
}

// --- node groups (access model) --------------------------------------------

pub async fn list_node_groups(pool: &PgPool) -> Result<Vec<NodeGroup>> {
    Ok(
        sqlx::query_as("SELECT * FROM node_groups ORDER BY is_default DESC, name")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn get_node_group(pool: &PgPool, id: Uuid) -> Result<Option<NodeGroup>> {
    Ok(sqlx::query_as("SELECT * FROM node_groups WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn create_node_group(pool: &PgPool, input: &NewNodeGroup) -> Result<NodeGroup> {
    Ok(
        sqlx::query_as("INSERT INTO node_groups (name, note) VALUES ($1, $2) RETURNING *")
            .bind(input.name.trim())
            .bind(input.note.trim())
            .fetch_one(pool)
            .await?,
    )
}

pub async fn update_node_group(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateNodeGroup,
) -> Result<Option<NodeGroup>> {
    let Some(cur) = get_node_group(pool, id).await? else {
        return Ok(None);
    };
    let name = input.name.clone().unwrap_or(cur.name);
    let note = input.note.clone().unwrap_or(cur.note);
    Ok(
        sqlx::query_as("UPDATE node_groups SET name = $2, note = $3 WHERE id = $1 RETURNING *")
            .bind(id)
            .bind(name.trim())
            .bind(note.trim())
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn delete_node_group(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(
        sqlx::query("DELETE FROM node_groups WHERE id = $1 AND NOT is_default")
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

pub async fn node_group_ids(pool: &PgPool, node_id: Uuid) -> Result<Vec<Uuid>> {
    Ok(
        sqlx::query_scalar("SELECT group_id FROM node_group_members WHERE node_id = $1")
            .bind(node_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn set_node_groups(pool: &PgPool, node_id: Uuid, group_ids: &[Uuid]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM node_group_members WHERE node_id = $1")
        .bind(node_id)
        .execute(&mut *tx)
        .await?;
    for gid in group_ids {
        sqlx::query(
            "INSERT INTO node_group_members (node_id, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(node_id)
        .bind(gid)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn user_group_ids(pool: &PgPool, user_id: Uuid) -> Result<Vec<Uuid>> {
    Ok(
        sqlx::query_scalar("SELECT group_id FROM user_group_access WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn set_user_groups(pool: &PgPool, user_id: Uuid, group_ids: &[Uuid]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_group_access WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for gid in group_ids {
        sqlx::query(
            "INSERT INTO user_group_access (user_id, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(gid)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// users who may reach node `node_id`: everyone if the node is ungrouped, else
/// users sharing a group with it. Used to build each node's spec.
pub async fn users_with_node_access(pool: &PgPool, node_id: Uuid) -> Result<Vec<User>> {
    decrypt_users(
        sqlx::query_as(
            "SELECT u.* FROM users u
             WHERE NOT EXISTS (SELECT 1 FROM node_group_members m WHERE m.node_id = $1)
                OR EXISTS (
                     SELECT 1 FROM node_group_members m
                     JOIN user_group_access a ON a.group_id = m.group_id
                     WHERE m.node_id = $1 AND a.user_id = u.id
                   )
             ORDER BY u.username",
        )
        .bind(node_id)
        .fetch_all(pool)
        .await?,
    )
}

/// nodes a user can reach (ungrouped ∪ shared-group) — used to target pushes.
pub async fn user_node_ids(pool: &PgPool, user_id: Uuid) -> Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT n.id FROM nodes n
         WHERE NOT EXISTS (SELECT 1 FROM node_group_members m WHERE m.node_id = n.id)
            OR EXISTS (
                 SELECT 1 FROM node_group_members m
                 JOIN user_group_access a ON a.group_id = m.group_id
                 WHERE m.node_id = n.id AND a.user_id = $1
               )",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub async fn subscription_endpoints(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<SubscriptionEndpoint>> {
    Ok(sqlx::query_as(
        "SELECT i.id AS inbound_id, n.name AS node_name, n.address, i.tag,
                i.type AS kind, i.core, i.listen_port, i.flow, i.tls_enabled,
                i.server_name, i.reality, i.reality_public_key,
                i.reality_short_ids,
                i.network, i.transport_path,
                -- direct→CDN failover: a confirmed-blocked endpoint with a
                -- fallback host is fronted via the CDN instead of being dropped.
                CASE WHEN i.reachable = false AND i.fallback_host IS NOT NULL AND i.fallback_host <> ''
                     THEN i.fallback_host ELSE i.transport_host END AS transport_host,
                i.transport_service_name, i.transport_mode,
                i.utls_fingerprint, i.ech, i.extra, n.extra_addresses
         FROM inbounds i
         JOIN nodes n ON n.id = i.node_id
         WHERE i.enabled AND n.enabled AND NOT n.maintenance
           AND (i.reachable IS DISTINCT FROM false
                OR (i.fallback_host IS NOT NULL AND i.fallback_host <> ''))
           AND (NOT EXISTS (SELECT 1 FROM node_group_members m WHERE m.node_id = n.id)
                OR EXISTS (
                     SELECT 1 FROM node_group_members m
                     JOIN user_group_access a ON a.group_id = m.group_id
                     WHERE m.node_id = n.id AND a.user_id = $1
                   ))
         ORDER BY n.name, i.listen_port",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// One endpoint descriptor for a specific inbound (the multihop exit), ignoring
/// per-user group access — chaining is operator-configured. `None` if the exit
/// inbound or its node is disabled.
pub async fn endpoint_for_inbound(
    pool: &PgPool,
    inbound_id: Uuid,
) -> Result<Option<SubscriptionEndpoint>> {
    Ok(sqlx::query_as(
        "SELECT i.id AS inbound_id, n.name AS node_name, n.address, i.tag,
                i.type AS kind, i.core, i.listen_port, i.flow, i.tls_enabled,
                i.server_name, i.reality, i.reality_public_key, i.reality_short_ids,
                i.network, i.transport_path, i.transport_host,
                i.transport_service_name, i.transport_mode,
                i.utls_fingerprint, i.ech, i.extra, n.extra_addresses
         FROM inbounds i JOIN nodes n ON n.id = i.node_id
         WHERE i.id = $1 AND i.enabled AND n.enabled AND NOT n.maintenance",
    )
    .bind(inbound_id)
    .fetch_optional(pool)
    .await?)
}

/// Chain credentials of every entry inbound that exits through `exit_id`, as
/// (label, uuid, decrypted password), to inject as users on the exit inbound.
pub async fn chain_users_for_exit(
    pool: &PgPool,
    exit_id: Uuid,
) -> Result<Vec<(String, String, Option<String>)>> {
    let rows: Vec<(Uuid, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, tag, chain_uuid, chain_password FROM inbounds
         WHERE upstream_inbound_id = $1 AND enabled AND chain_uuid IS NOT NULL",
    )
    .bind(exit_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, tag, uuid, pw)| {
            let password = pw.map(|p| secret::decrypt(&p)).transpose()?;
            Ok((
                format!("chain-{tag}-{id}"),
                uuid.unwrap_or_default(),
                password,
            ))
        })
        .collect()
}

fn traffic_delta(
    previous: Option<(i64, i64, String)>,
    epoch: &str,
    cur_up: i64,
    cur_down: i64,
) -> (i64, i64) {
    let Some((last_up, last_down, last_epoch)) = previous else {
        return (0, 0);
    };
    if last_epoch != epoch {
        return (0, 0);
    }
    (
        cur_up
            .checked_sub(last_up)
            .filter(|delta| *delta >= 0)
            .unwrap_or(0),
        cur_down
            .checked_sub(last_down)
            .filter(|delta| *delta >= 0)
            .unwrap_or(0),
    )
}

pub async fn record_traffic(
    pool: &PgPool,
    node_id: Uuid,
    user_id: Uuid,
    core: &str,
    epoch: &str,
    cur_up: i64,
    cur_down: i64,
) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let before: (i64, i64) = sqlx::query_as(
        "SELECT traffic_limit_bytes, used_traffic_bytes FROM users
         WHERE id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;

    // Lock the source row before calculating the restart-safe delta. A new
    // counter epoch is a baseline, never historical usage.
    let previous: Option<(i64, i64, String)> = sqlx::query_as(
        "SELECT last_up, last_down, last_epoch FROM node_user_traffic
         WHERE node_id = $1 AND user_id = $2 AND core = $3 FOR UPDATE",
    )
    .bind(node_id)
    .bind(user_id)
    .bind(core)
    .fetch_optional(&mut *tx)
    .await?;
    let (delta_up, delta_down) = traffic_delta(previous, epoch, cur_up, cur_down);

    sqlx::query(
        "INSERT INTO node_user_traffic
           (node_id, user_id, core, up_bytes, down_bytes, last_up, last_down, last_epoch)
         VALUES ($1, $2, $6, 0, 0, $3, $4, $5)
         ON CONFLICT (node_id, user_id, core) DO UPDATE SET
           up_bytes = node_user_traffic.up_bytes + CASE
             WHEN node_user_traffic.last_epoch = $5 AND $3 >= node_user_traffic.last_up
             THEN $3 - node_user_traffic.last_up ELSE 0 END,
           down_bytes = node_user_traffic.down_bytes + CASE
             WHEN node_user_traffic.last_epoch = $5 AND $4 >= node_user_traffic.last_down
             THEN $4 - node_user_traffic.last_down ELSE 0 END,
           last_up = $3, last_down = $4, last_epoch = $5, updated_at = now()",
    )
    .bind(node_id)
    .bind(user_id)
    .bind(cur_up)
    .bind(cur_down)
    .bind(epoch)
    .bind(core)
    .execute(&mut *tx)
    .await?;

    if delta_up > 0 || delta_down > 0 {
        sqlx::query(
            "INSERT INTO traffic_usage_hourly
               (bucket, node_id, user_id, core, up_bytes, down_bytes, sample_count)
             VALUES (date_trunc('hour', now()), $1, $2, $3, $4, $5, 1)
             ON CONFLICT (bucket, node_id, user_id, core) DO UPDATE SET
               up_bytes = traffic_usage_hourly.up_bytes + EXCLUDED.up_bytes,
               down_bytes = traffic_usage_hourly.down_bytes + EXCLUDED.down_bytes,
               sample_count = traffic_usage_hourly.sample_count + 1",
        )
        .bind(node_id)
        .bind(user_id)
        .bind(core)
        .bind(delta_up)
        .bind(delta_down)
        .execute(&mut *tx)
        .await?;
    }

    let after: (i64,) = sqlx::query_as(
        "UPDATE users SET used_traffic_bytes = (
           SELECT COALESCE(SUM(up_bytes + down_bytes), 0)
           FROM node_user_traffic WHERE user_id = $1)
         WHERE id = $1 RETURNING used_traffic_bytes",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    let limit = before.0;
    Ok(limit > 0 && before.1 < limit && after.0 >= limit)
}

pub async fn traffic_totals(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    node_id: Option<Uuid>,
    user_id: Option<Uuid>,
    core: Option<&str>,
    creator: Option<Uuid>,
) -> Result<(i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(sum(h.up_bytes), 0)::bigint,
                COALESCE(sum(h.down_bytes), 0)::bigint
         FROM traffic_usage_hourly h
         JOIN users u ON u.id = h.user_id
         WHERE h.bucket >= $1 AND h.bucket < $2
           AND ($3::uuid IS NULL OR h.node_id = $3)
           AND ($4::uuid IS NULL OR h.user_id = $4)
           AND ($5::text IS NULL OR h.core = $5)
           AND ($6::uuid IS NULL OR u.created_by = $6)",
    )
    .bind(from)
    .bind(to)
    .bind(node_id)
    .bind(user_id)
    .bind(core)
    .bind(creator)
    .fetch_one(pool)
    .await?)
}

pub async fn traffic_series(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: &str,
    node_id: Option<Uuid>,
    user_id: Option<Uuid>,
    core: Option<&str>,
    creator: Option<Uuid>,
) -> Result<Vec<TrafficSeriesPoint>> {
    Ok(sqlx::query_as(
        "SELECT date_trunc($7, h.bucket) AS bucket,
                COALESCE(sum(h.up_bytes), 0)::bigint AS up_bytes,
                COALESCE(sum(h.down_bytes), 0)::bigint AS down_bytes
         FROM traffic_usage_hourly h
         JOIN users u ON u.id = h.user_id
         WHERE h.bucket >= $1 AND h.bucket < $2
           AND ($3::uuid IS NULL OR h.node_id = $3)
           AND ($4::uuid IS NULL OR h.user_id = $4)
           AND ($5::text IS NULL OR h.core = $5)
           AND ($6::uuid IS NULL OR u.created_by = $6)
         GROUP BY 1 ORDER BY 1",
    )
    .bind(from)
    .bind(to)
    .bind(node_id)
    .bind(user_id)
    .bind(core)
    .bind(creator)
    .bind(bucket)
    .fetch_all(pool)
    .await?)
}

pub async fn traffic_top_users(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    node_id: Option<Uuid>,
    user_id: Option<Uuid>,
    core: Option<&str>,
    creator: Option<Uuid>,
) -> Result<Vec<TrafficRank>> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.username AS name,
                COALESCE(sum(h.up_bytes), 0)::bigint AS up_bytes,
                COALESCE(sum(h.down_bytes), 0)::bigint AS down_bytes
         FROM traffic_usage_hourly h
         JOIN users u ON u.id = h.user_id
         WHERE h.bucket >= $1 AND h.bucket < $2
           AND ($3::uuid IS NULL OR h.node_id = $3)
           AND ($4::uuid IS NULL OR h.user_id = $4)
           AND ($5::text IS NULL OR h.core = $5)
           AND ($6::uuid IS NULL OR u.created_by = $6)
         GROUP BY u.id, u.username
         ORDER BY sum(h.up_bytes + h.down_bytes) DESC, u.username
         LIMIT 10",
    )
    .bind(from)
    .bind(to)
    .bind(node_id)
    .bind(user_id)
    .bind(core)
    .bind(creator)
    .fetch_all(pool)
    .await?)
}

pub async fn traffic_top_nodes(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    node_id: Option<Uuid>,
    user_id: Option<Uuid>,
    core: Option<&str>,
) -> Result<Vec<TrafficRank>> {
    Ok(sqlx::query_as(
        "SELECT n.id, n.name,
                COALESCE(sum(h.up_bytes), 0)::bigint AS up_bytes,
                COALESCE(sum(h.down_bytes), 0)::bigint AS down_bytes
         FROM traffic_usage_hourly h
         JOIN nodes n ON n.id = h.node_id
         WHERE h.bucket >= $1 AND h.bucket < $2
           AND ($3::uuid IS NULL OR h.node_id = $3)
           AND ($4::uuid IS NULL OR h.user_id = $4)
           AND ($5::text IS NULL OR h.core = $5)
         GROUP BY n.id, n.name
         ORDER BY sum(h.up_bytes + h.down_bytes) DESC, n.name
         LIMIT 10",
    )
    .bind(from)
    .bind(to)
    .bind(node_id)
    .bind(user_id)
    .bind(core)
    .fetch_all(pool)
    .await?)
}

pub async fn traffic_by_core(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    node_id: Option<Uuid>,
    user_id: Option<Uuid>,
    core: Option<&str>,
    creator: Option<Uuid>,
) -> Result<Vec<TrafficCoreBreakdown>> {
    Ok(sqlx::query_as(
        "SELECT h.core,
                COALESCE(sum(h.up_bytes), 0)::bigint AS up_bytes,
                COALESCE(sum(h.down_bytes), 0)::bigint AS down_bytes
         FROM traffic_usage_hourly h
         JOIN users u ON u.id = h.user_id
         WHERE h.bucket >= $1 AND h.bucket < $2
           AND ($3::uuid IS NULL OR h.node_id = $3)
           AND ($4::uuid IS NULL OR h.user_id = $4)
           AND ($5::text IS NULL OR h.core = $5)
           AND ($6::uuid IS NULL OR u.created_by = $6)
         GROUP BY h.core ORDER BY h.core",
    )
    .bind(from)
    .bind(to)
    .bind(node_id)
    .bind(user_id)
    .bind(core)
    .bind(creator)
    .fetch_all(pool)
    .await?)
}

pub async fn fleet_health_summary(pool: &PgPool) -> Result<FleetHealthSummary> {
    Ok(sqlx::query_as(
        "SELECT
           count(*) FILTER (WHERE enabled)::bigint AS nodes_total,
           count(*) FILTER (WHERE enabled AND last_seen >= now() - interval '2 minutes')::bigint AS nodes_online,
           count(*) FILTER (WHERE enabled AND last_push_status = 'failed')::bigint AS failed_pushes,
           (SELECT count(*)::bigint FROM inbounds WHERE enabled AND reachable = false) AS unreachable_endpoints
         FROM nodes",
    )
    .fetch_one(pool)
    .await?)
}

pub async fn delete_traffic_history_before(pool: &PgPool, cutoff: DateTime<Utc>) -> Result<u64> {
    Ok(
        sqlx::query("DELETE FROM traffic_usage_hourly WHERE bucket < $1")
            .bind(cutoff)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

// --- multi-master HA (leader lease + instance roster) -----------------------

/// Atomically take or renew the leader lease. Returns true when this holder owns
/// it afterwards. The `WHERE` on the upsert is what makes this safe: another
/// instance can only take over once the current lease has actually expired.
pub async fn ha_try_acquire(pool: &PgPool, holder: Uuid, ttl_secs: i64) -> Result<bool> {
    let winner: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO ha_leader (id, holder, expires_at)
         VALUES ('master', $1, now() + make_interval(secs => $2::int))
         ON CONFLICT (id) DO UPDATE
           SET holder = EXCLUDED.holder,
               acquired_at = CASE WHEN ha_leader.holder = EXCLUDED.holder
                                  THEN ha_leader.acquired_at ELSE now() END,
               renewed_at = now(),
               expires_at = EXCLUDED.expires_at
           WHERE ha_leader.holder = EXCLUDED.holder OR ha_leader.expires_at < now()
         RETURNING holder",
    )
    .bind(holder)
    .bind(ttl_secs)
    .fetch_optional(pool)
    .await?;
    Ok(winner == Some(holder))
}

pub async fn ha_heartbeat(
    pool: &PgPool,
    instance: Uuid,
    hostname: &str,
    version: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO ha_instances (instance_id, hostname, version)
         VALUES ($1, $2, $3)
         ON CONFLICT (instance_id) DO UPDATE
           SET hostname = EXCLUDED.hostname, version = EXCLUDED.version, last_seen = now()",
    )
    .bind(instance)
    .bind(hostname)
    .bind(version)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn ha_prune_instances(pool: &PgPool, older_than_secs: i64) -> Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM ha_instances WHERE last_seen < now() - make_interval(secs => $1::int)",
    )
    .bind(older_than_secs.max(60))
    .execute(pool)
    .await?
    .rows_affected())
}

/// (holder, expires_at) of the live lease, if any instance currently holds it.
pub async fn ha_leader(pool: &PgPool) -> Result<Option<(Uuid, DateTime<Utc>)>> {
    Ok(sqlx::query_as(
        "SELECT holder, expires_at FROM ha_leader WHERE id = 'master' AND expires_at > now()",
    )
    .fetch_optional(pool)
    .await?)
}

/// Instance roster for the panel: (instance_id, hostname, version, started_at, last_seen).
pub async fn ha_instances(
    pool: &PgPool,
) -> Result<Vec<(Uuid, String, String, DateTime<Utc>, DateTime<Utc>)>> {
    Ok(sqlx::query_as(
        "SELECT instance_id, hostname, version, started_at, last_seen
         FROM ha_instances ORDER BY started_at",
    )
    .fetch_all(pool)
    .await?)
}

// --- managed external services (MTProto / NaiveProxy) -----------------------

/// Generate the per-kind secret at create time: an MTProto `ee`-secret (fake-TLS)
/// or a NaiveProxy password. Returned already encrypted for storage.
fn service_secret(kind: &str, config: &Json) -> Result<Option<String>> {
    let secret = match kind {
        "mtproto" => {
            let mut raw = [0u8; 16];
            getrandom::getrandom(&mut raw).map_err(|e| anyhow::anyhow!("rng: {e}"))?;
            let host = config
                .get("host")
                .and_then(Json::as_str)
                .unwrap_or("www.cloudflare.com");
            let hex_rand: String = raw.iter().map(|b| format!("{b:02x}")).collect();
            let hex_host: String = host.bytes().map(|b| format!("{b:02x}")).collect();
            // "ee" prefix = fake-TLS (padded) MTProto secret.
            format!("ee{hex_rand}{hex_host}")
        }
        "naive" => {
            let mut raw = [0u8; 16];
            getrandom::getrandom(&mut raw).map_err(|e| anyhow::anyhow!("rng: {e}"))?;
            raw.iter().map(|b| format!("{b:02x}")).collect()
        }
        _ => return Ok(None),
    };
    Ok(Some(secret::encrypt(&secret)?))
}

pub async fn create_node_service(pool: &PgPool, input: NewNodeService) -> Result<NodeService> {
    let secret = service_secret(&input.kind, &input.config)?;
    let svc: NodeService = sqlx::query_as(
        "INSERT INTO node_services (node_id, kind, name, listen_port, secret, config)
         VALUES ($1,$2,$3,$4,$5,$6) RETURNING *",
    )
    .bind(input.node_id)
    .bind(&input.kind)
    .bind(input.name.trim())
    .bind(input.listen_port)
    .bind(secret)
    .bind(sqlx::types::Json(input.config))
    .fetch_one(pool)
    .await?;
    decrypt_service(svc)
}

pub async fn list_node_services(pool: &PgPool, node_id: Uuid) -> Result<Vec<NodeService>> {
    let rows: Vec<NodeService> =
        sqlx::query_as("SELECT * FROM node_services WHERE node_id = $1 ORDER BY created_at")
            .bind(node_id)
            .fetch_all(pool)
            .await?;
    rows.into_iter().map(decrypt_service).collect()
}

pub async fn get_node_service(pool: &PgPool, id: Uuid) -> Result<Option<NodeService>> {
    match sqlx::query_as::<_, NodeService>("SELECT * FROM node_services WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
    {
        Some(s) => Ok(Some(decrypt_service(s)?)),
        None => Ok(None),
    }
}

pub async fn update_node_service(
    pool: &PgPool,
    id: Uuid,
    input: UpdateNodeService,
) -> Result<Option<NodeService>> {
    let row: Option<NodeService> = sqlx::query_as(
        "UPDATE node_services SET
           name = COALESCE($2, name),
           listen_port = COALESCE($3, listen_port),
           enabled = COALESCE($4, enabled),
           config = COALESCE($5, config),
           updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(input.name)
    .bind(input.listen_port)
    .bind(input.enabled)
    .bind(input.config.map(sqlx::types::Json))
    .fetch_optional(pool)
    .await?;
    match row {
        Some(s) => Ok(Some(decrypt_service(s)?)),
        None => Ok(None),
    }
}

pub async fn delete_node_service(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM node_services WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

pub async fn enabled_node_services(pool: &PgPool, node_id: Uuid) -> Result<Vec<NodeService>> {
    let rows: Vec<NodeService> = sqlx::query_as(
        "SELECT * FROM node_services WHERE node_id = $1 AND enabled ORDER BY created_at",
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(decrypt_service).collect()
}

/// Enabled services on nodes this user can reach (same group access as inbounds),
/// with the node address, for the subscription's client links.
pub async fn node_services_for_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<(NodeService, String)>> {
    let rows: Vec<(NodeService, String)> = {
        // fetch services + node address separately to avoid a tuple FromRow on a struct.
        let svcs: Vec<NodeService> = sqlx::query_as(
            "SELECT s.* FROM node_services s
             JOIN nodes n ON n.id = s.node_id
             WHERE s.enabled AND n.enabled
               AND (NOT EXISTS (SELECT 1 FROM node_group_members m WHERE m.node_id = s.node_id)
                    OR EXISTS (
                         SELECT 1 FROM node_group_members m
                         JOIN user_group_access a ON a.group_id = m.group_id
                         WHERE m.node_id = s.node_id AND a.user_id = $1))
             ORDER BY s.created_at",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;
        let mut out = Vec::with_capacity(svcs.len());
        for s in svcs {
            let addr: String = sqlx::query_scalar("SELECT address FROM nodes WHERE id = $1")
                .bind(s.node_id)
                .fetch_one(pool)
                .await?;
            out.push((decrypt_service(s)?, addr));
        }
        out
    };
    Ok(rows)
}

fn decrypt_service(mut svc: NodeService) -> Result<NodeService> {
    if let Some(sec) = svc.secret.take() {
        svc.secret = Some(secret::decrypt(&sec)?);
    }
    Ok(svc)
}

// --- WireGuard / AmneziaWG --------------------------------------------------

pub async fn create_wg_interface(pool: &PgPool, input: NewWgInterface) -> Result<WgInterface> {
    let kp = crate::wg::generate()?;
    let amnezia_params = if input.amnezia {
        serde_json::to_value(crate::wg::AmneziaParams::generate()?)?
    } else {
        serde_json::json!({})
    };
    let private_enc = secret::encrypt(&kp.private_key)?;
    let iface: WgInterface = sqlx::query_as(
        "INSERT INTO wg_interfaces
           (node_id, name, listen_port, private_key, public_key, address_cidr, dns, mtu,
            amnezia, amnezia_params, endpoint_host)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING *",
    )
    .bind(input.node_id)
    .bind(input.name.trim())
    .bind(input.listen_port)
    .bind(private_enc)
    .bind(kp.public_key)
    .bind(input.address_cidr.trim())
    .bind(input.dns.trim())
    .bind(input.mtu)
    .bind(input.amnezia)
    .bind(sqlx::types::Json(amnezia_params))
    .bind(input.endpoint_host.map(|h| h.trim().to_string()))
    .fetch_one(pool)
    .await?;
    Ok(decrypt_wg_interface(iface)?)
}

pub async fn list_wg_interfaces(pool: &PgPool, node_id: Uuid) -> Result<Vec<WgInterface>> {
    let rows: Vec<WgInterface> =
        sqlx::query_as("SELECT * FROM wg_interfaces WHERE node_id = $1 ORDER BY created_at")
            .bind(node_id)
            .fetch_all(pool)
            .await?;
    rows.into_iter().map(decrypt_wg_interface).collect()
}

pub async fn get_wg_interface(pool: &PgPool, id: Uuid) -> Result<Option<WgInterface>> {
    match sqlx::query_as::<_, WgInterface>("SELECT * FROM wg_interfaces WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
    {
        Some(iface) => Ok(Some(decrypt_wg_interface(iface)?)),
        None => Ok(None),
    }
}

pub async fn update_wg_interface(
    pool: &PgPool,
    id: Uuid,
    input: UpdateWgInterface,
) -> Result<Option<WgInterface>> {
    let (ep_set, ep_val) = patch_parts(input.endpoint_host);
    let row: Option<WgInterface> = sqlx::query_as(
        "UPDATE wg_interfaces SET
           name = COALESCE($2, name),
           listen_port = COALESCE($3, listen_port),
           dns = COALESCE($4, dns),
           mtu = COALESCE($5, mtu),
           enabled = COALESCE($6, enabled),
           endpoint_host = CASE WHEN $7 THEN $8 ELSE endpoint_host END,
           updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(input.name)
    .bind(input.listen_port)
    .bind(input.dns)
    .bind(input.mtu)
    .bind(input.enabled)
    .bind(ep_set)
    .bind(ep_val)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(iface) => Ok(Some(decrypt_wg_interface(iface)?)),
        None => Ok(None),
    }
}

pub async fn delete_wg_interface(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM wg_interfaces WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// Enabled interfaces for a node (spec build / reconcile).
pub async fn enabled_wg_interfaces(pool: &PgPool, node_id: Uuid) -> Result<Vec<WgInterface>> {
    let rows: Vec<WgInterface> = sqlx::query_as(
        "SELECT * FROM wg_interfaces WHERE node_id = $1 AND enabled ORDER BY created_at",
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(decrypt_wg_interface).collect()
}

pub async fn wg_peers_for_interface(pool: &PgPool, interface_id: Uuid) -> Result<Vec<WgPeer>> {
    let rows: Vec<WgPeer> = sqlx::query_as("SELECT * FROM wg_peers WHERE interface_id = $1")
        .bind(interface_id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(decrypt_wg_peer).collect()
}

/// Get-or-create the peer for one user on one interface, allocating the next
/// free `/32`. Returns the peer with its private key decrypted.
pub async fn ensure_wg_peer(pool: &PgPool, iface: &WgInterface, user_id: Uuid) -> Result<WgPeer> {
    if let Some(existing) = sqlx::query_as::<_, WgPeer>(
        "SELECT * FROM wg_peers WHERE interface_id = $1 AND user_id = $2",
    )
    .bind(iface.id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?
    {
        return decrypt_wg_peer(existing);
    }
    let taken: Vec<String> =
        sqlx::query_scalar("SELECT address FROM wg_peers WHERE interface_id = $1")
            .bind(iface.id)
            .fetch_all(pool)
            .await?;
    let taken_ips: Vec<std::net::Ipv4Addr> = taken.iter().filter_map(|a| a.parse().ok()).collect();
    let ip = crate::wg::allocate_ip(&iface.address_cidr, &taken_ips)?;
    let kp = crate::wg::generate()?;
    let private_enc = secret::encrypt(&kp.private_key)?;
    let peer: WgPeer = sqlx::query_as(
        "INSERT INTO wg_peers (interface_id, user_id, private_key, public_key, address)
         VALUES ($1,$2,$3,$4,$5) RETURNING *",
    )
    .bind(iface.id)
    .bind(user_id)
    .bind(private_enc)
    .bind(kp.public_key)
    .bind(ip.to_string())
    .fetch_one(pool)
    .await?;
    decrypt_wg_peer(peer)
}

/// Enabled WG interfaces on nodes this user can reach (same group access as
/// inbounds). Private keys are decrypted. The caller resolves the endpoint host.
pub async fn wg_interfaces_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<WgInterface>> {
    let rows: Vec<WgInterface> = sqlx::query_as(
        "SELECT w.* FROM wg_interfaces w
         JOIN nodes n ON n.id = w.node_id
         WHERE w.enabled AND n.enabled
           AND (NOT EXISTS (SELECT 1 FROM node_group_members m WHERE m.node_id = w.node_id)
                OR EXISTS (
                     SELECT 1 FROM node_group_members m
                     JOIN user_group_access a ON a.group_id = m.group_id
                     WHERE m.node_id = w.node_id AND a.user_id = $1))
         ORDER BY w.created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(decrypt_wg_interface).collect()
}

fn decrypt_wg_interface(mut iface: WgInterface) -> Result<WgInterface> {
    iface.private_key = secret::decrypt(&iface.private_key)?;
    Ok(iface)
}

fn decrypt_wg_peer(mut peer: WgPeer) -> Result<WgPeer> {
    peer.private_key = secret::decrypt(&peer.private_key)?;
    Ok(peer)
}

/// All encrypted WG private keys (interfaces + peers) as (table, id, enc), for
/// the secret rekey/reencrypt passes.
async fn wg_private_keys(pool: &PgPool) -> Result<Vec<(&'static str, Uuid, String)>> {
    let mut out = Vec::new();
    for (id, key) in
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, private_key FROM wg_interfaces")
            .fetch_all(pool)
            .await?
    {
        out.push(("wg_interfaces", id, key));
    }
    for (id, key) in sqlx::query_as::<_, (Uuid, String)>("SELECT id, private_key FROM wg_peers")
        .fetch_all(pool)
        .await?
    {
        out.push(("wg_peers", id, key));
    }
    Ok(out)
}

/// Users with an active device limit (username -> id, limit), for the
/// anti-sharing monitor. Only enabled users are worth checking.
pub async fn device_limited_users(pool: &PgPool) -> Result<Vec<(Uuid, String, i32)>> {
    Ok(sqlx::query_as(
        "SELECT id, username, device_limit FROM users
         WHERE device_limit > 0 AND enabled",
    )
    .fetch_all(pool)
    .await?)
}

// --- anomaly detection & status-page uptime ---------------------------------

/// Flag users whose most recent completed hour of traffic is at least
/// `factor_pct`% of their average active-hour usage over the baseline window,
/// and above an absolute floor. Only users with `min_history` active hours in
/// the window are considered, so new accounts don't false-alarm.
pub async fn detect_traffic_anomalies(
    pool: &PgPool,
    factor_pct: i64,
    min_bytes: i64,
    baseline_hours: i64,
    min_history: i64,
) -> Result<Vec<TrafficAnomaly>> {
    Ok(sqlx::query_as(
        "WITH recent AS (
           SELECT user_id, SUM(up_bytes + down_bytes)::bigint AS bytes
           FROM traffic_usage_hourly
           WHERE bucket = date_trunc('hour', now() - interval '1 hour')
           GROUP BY user_id
         ),
         hist AS (
           SELECT user_id, bucket, SUM(up_bytes + down_bytes)::bigint AS bytes
           FROM traffic_usage_hourly
           WHERE bucket >= date_trunc('hour', now()) - make_interval(hours => $3::int)
             AND bucket <  date_trunc('hour', now() - interval '1 hour')
           GROUP BY user_id, bucket
         ),
         baseline AS (
           SELECT user_id, avg(bytes)::bigint AS avg_bytes, count(*)::bigint AS hours
           FROM hist GROUP BY user_id
         )
         SELECT r.user_id, u.username, r.bytes AS last_bytes, b.avg_bytes AS baseline_bytes
         FROM recent r
         JOIN baseline b ON b.user_id = r.user_id
         JOIN users u ON u.id = r.user_id
         WHERE b.hours >= $4
           AND b.avg_bytes > 0
           AND r.bytes >= $2
           AND r.bytes::numeric >= b.avg_bytes::numeric * ($1::numeric / 100.0)
         ORDER BY r.bytes DESC
         LIMIT 200",
    )
    .bind(factor_pct)
    .bind(min_bytes)
    .bind(baseline_hours)
    .bind(min_history)
    .fetch_all(pool)
    .await?)
}

/// Record one online/offline probe per enabled, non-maintenance node from
/// `last_seen` freshness. Cheap DB-only sample; no agent round-trip.
pub async fn sample_node_status(pool: &PgPool) -> Result<u64> {
    Ok(sqlx::query(
        "INSERT INTO node_status_samples (node_id, online)
         SELECT id, (last_seen IS NOT NULL AND last_seen > now() - interval '2 minutes')
         FROM nodes WHERE enabled AND NOT maintenance",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

pub async fn prune_node_status_samples(pool: &PgPool, keep_days: i64) -> Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM node_status_samples WHERE sampled_at < now() - make_interval(days => $1::int)",
    )
    .bind(keep_days.max(1))
    .execute(pool)
    .await?
    .rows_affected())
}

/// Per-node availability ratio (0..1) and sample count over the last `hours`.
pub async fn node_uptime(pool: &PgPool, hours: i64) -> Result<Vec<NodeUptime>> {
    Ok(sqlx::query_as(
        "SELECT node_id,
                avg(online::int)::float8 AS ratio,
                count(*)::bigint AS samples
         FROM node_status_samples
         WHERE sampled_at > now() - make_interval(hours => $1::int)
         GROUP BY node_id",
    )
    .bind(hours.max(1))
    .fetch_all(pool)
    .await?)
}

/// Recent availability events for the public incident timeline.
pub async fn recent_incidents(pool: &PgPool, days: i64, limit: i64) -> Result<Vec<StatusIncident>> {
    Ok(sqlx::query_as(
        "SELECT title, severity, created_at, last_seen_at, occurrence_count
         FROM system_notifications
         WHERE event_type IN ('node_down', 'push_failed', 'cert_expiry')
           AND created_at > now() - make_interval(days => $1::int)
         ORDER BY created_at DESC
         LIMIT $2",
    )
    .bind(days.max(1))
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await?)
}

// --- P0 operator identity, durable state and enrollment --------------------

pub async fn count_enabled_admins(pool: &PgPool) -> Result<i64> {
    Ok(
        sqlx::query_as::<_, (i64,)>("SELECT count(*) FROM admins WHERE enabled")
            .fetch_one(pool)
            .await?
            .0,
    )
}

pub async fn create_admin(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
    role: &str,
    max_users: i32,
    user_traffic_ceiling_bytes: i64,
    traffic_limit_bytes: i64,
    commission_percent: i32,
) -> Result<Admin> {
    Ok(sqlx::query_as(
        "INSERT INTO admins
           (username, password_hash, role, max_users, user_traffic_ceiling_bytes,
            traffic_limit_bytes, commission_percent)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
    )
    .bind(username.trim())
    .bind(password_hash)
    .bind(role)
    .bind(max_users)
    .bind(user_traffic_ceiling_bytes)
    .bind(traffic_limit_bytes)
    .bind(commission_percent)
    .fetch_one(pool)
    .await?)
}

pub async fn list_admins(pool: &PgPool) -> Result<Vec<Admin>> {
    Ok(
        sqlx::query_as("SELECT * FROM admins ORDER BY lower(username)")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn get_admin(pool: &PgPool, id: Uuid) -> Result<Option<Admin>> {
    Ok(sqlx::query_as("SELECT * FROM admins WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

pub async fn get_admin_by_username(pool: &PgPool, username: &str) -> Result<Option<Admin>> {
    Ok(
        sqlx::query_as("SELECT * FROM admins WHERE lower(username) = lower($1)")
            .bind(username.trim())
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn update_admin(
    pool: &PgPool,
    id: Uuid,
    role: Option<&str>,
    enabled: Option<bool>,
    password_hash: Option<&str>,
    max_users: Option<i32>,
    user_traffic_ceiling_bytes: Option<i64>,
    traffic_limit_bytes: Option<i64>,
    commission_percent: Option<i32>,
) -> Result<Option<Admin>> {
    Ok(sqlx::query_as(
        "UPDATE admins SET role = COALESCE($2, role),
           enabled = COALESCE($3, enabled),
           password_hash = COALESCE($4, password_hash),
           max_users = COALESCE($5, max_users),
           user_traffic_ceiling_bytes = COALESCE($6, user_traffic_ceiling_bytes),
           traffic_limit_bytes = COALESCE($7, traffic_limit_bytes),
           commission_percent = COALESCE($8, commission_percent)
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(role)
    .bind(enabled)
    .bind(password_hash)
    .bind(max_users)
    .bind(user_traffic_ceiling_bytes)
    .bind(traffic_limit_bytes)
    .bind(commission_percent)
    .fetch_optional(pool)
    .await?)
}

// --- resellers (scoped sub-admins) -----------------------------------------

/// Groups a reseller is entitled to grant to its users.
pub async fn reseller_group_ids(pool: &PgPool, admin_id: Uuid) -> Result<Vec<Uuid>> {
    Ok(
        sqlx::query_scalar("SELECT group_id FROM reseller_groups WHERE admin_id = $1")
            .bind(admin_id)
            .fetch_all(pool)
            .await?,
    )
}

/// Full-replace a reseller's group entitlement.
pub async fn set_reseller_groups(pool: &PgPool, admin_id: Uuid, group_ids: &[Uuid]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM reseller_groups WHERE admin_id = $1")
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
    for group_id in group_ids {
        sqlx::query(
            "INSERT INTO reseller_groups (admin_id, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(admin_id)
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// How many users a reseller currently owns (for max_users enforcement).
pub async fn count_users_for_creator(pool: &PgPool, admin_id: Uuid) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM users WHERE created_by = $1")
            .bind(admin_id)
            .fetch_one(pool)
            .await?,
    )
}

/// Total traffic used across a reseller's own users (for the budget cap).
pub async fn reseller_traffic_used(pool: &PgPool, admin_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(sum(used_traffic_bytes), 0)::bigint FROM users WHERE created_by = $1",
    )
    .bind(admin_id)
    .fetch_one(pool)
    .await?)
}

/// Users owned by a given creator (a reseller sees only these).
pub async fn list_users_for_creator(pool: &PgPool, admin_id: Uuid) -> Result<Vec<User>> {
    decrypt_users(
        sqlx::query_as("SELECT * FROM users WHERE created_by = $1 ORDER BY username")
            .bind(admin_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn create_admin_session(
    pool: &PgPool,
    admin_id: Uuid,
    token_hash: &[u8],
    expires_at: DateTime<Utc>,
    user_agent: Option<&str>,
    remote_addr: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO admin_sessions
           (admin_id, token_hash, expires_at, user_agent, remote_addr)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(admin_id)
    .bind(token_hash)
    .bind(expires_at)
    .bind(user_agent)
    .bind(remote_addr)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE admins SET last_login_at = now() WHERE id = $1")
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn admin_for_session(pool: &PgPool, token_hash: &[u8]) -> Result<Option<Admin>> {
    let admin = sqlx::query_as(
        "SELECT a.* FROM admins a
         JOIN admin_sessions s ON s.admin_id = a.id
         WHERE s.token_hash = $1 AND s.expires_at > now() AND a.enabled",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    if admin.is_some() {
        sqlx::query(
            "UPDATE admin_sessions SET last_seen_at = now()
             WHERE token_hash = $1 AND last_seen_at < now() - interval '5 minutes'",
        )
        .bind(token_hash)
        .execute(pool)
        .await?;
    }
    Ok(admin)
}

pub async fn delete_admin_session(pool: &PgPool, token_hash: &[u8]) -> Result<bool> {
    Ok(
        sqlx::query("DELETE FROM admin_sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

pub async fn delete_admin_sessions(pool: &PgPool, admin_id: Uuid) -> Result<u64> {
    Ok(
        sqlx::query("DELETE FROM admin_sessions WHERE admin_id = $1")
            .bind(admin_id)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

pub async fn list_admin_sessions(pool: &PgPool, admin_id: Uuid) -> Result<Vec<AdminSession>> {
    Ok(sqlx::query_as(
        "SELECT s.id, s.admin_id, a.username, s.expires_at, s.last_seen_at,
                s.user_agent, s.remote_addr, s.created_at
         FROM admin_sessions s JOIN admins a ON a.id = s.admin_id
         WHERE s.admin_id = $1 AND s.expires_at > now()
         ORDER BY s.last_seen_at DESC",
    )
    .bind(admin_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_admin_session(pool: &PgPool, id: Uuid) -> Result<Option<AdminSession>> {
    Ok(sqlx::query_as(
        "SELECT s.id, s.admin_id, a.username, s.expires_at, s.last_seen_at,
                s.user_agent, s.remote_addr, s.created_at
         FROM admin_sessions s JOIN admins a ON a.id = s.admin_id
         WHERE s.id = $1 AND s.expires_at > now()",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn admin_session_id_for_hash(pool: &PgPool, token_hash: &[u8]) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM admin_sessions WHERE token_hash = $1 AND expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_admin_session_by_id(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM admin_sessions WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

pub async fn delete_other_admin_sessions(
    pool: &PgPool,
    admin_id: Uuid,
    current_hash: &[u8],
) -> Result<u64> {
    Ok(sqlx::query(
        "DELETE FROM admin_sessions
             WHERE admin_id = $1 AND token_hash <> $2 AND expires_at > now()",
    )
    .bind(admin_id)
    .bind(current_hash)
    .execute(pool)
    .await?
    .rows_affected())
}

pub async fn record_admin_login_event(
    pool: &PgPool,
    admin_id: Option<Uuid>,
    username: &str,
    outcome: &str,
    remote_addr: Option<&str>,
    user_agent: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO admin_login_events
           (admin_id, username, outcome, remote_addr, user_agent)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(admin_id)
    .bind(username)
    .bind(outcome)
    .bind(remote_addr)
    .bind(user_agent)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= now()")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM admin_login_events WHERE created_at < now() - interval '90 days'")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn list_admin_login_events(
    pool: &PgPool,
    admin_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AdminLoginEvent>> {
    if let Some(admin_id) = admin_id {
        Ok(sqlx::query_as(
            "SELECT * FROM admin_login_events WHERE admin_id = $1
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(admin_id)
        .bind(limit)
        .fetch_all(pool)
        .await?)
    } else {
        Ok(
            sqlx::query_as("SELECT * FROM admin_login_events ORDER BY created_at DESC LIMIT $1")
                .bind(limit)
                .fetch_all(pool)
                .await?,
        )
    }
}

pub async fn record_audit(
    pool: &PgPool,
    actor_admin_id: Option<Uuid>,
    actor_name: Option<&str>,
    action: &str,
    resource_type: &str,
    resource_id: Option<&str>,
    remote_addr: Option<&str>,
    details: Json,
) -> Result<()> {
    // serialize audit writers so the hash chain stays linear, then insert and
    // stamp the tamper-evident chain hash.
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(4823001)")
        .execute(&mut *tx)
        .await?;
    let prev: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT entry_hash FROM audit_events ORDER BY id DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
    let details_string = details.to_string();
    let (id, created): (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO audit_events
           (actor_admin_id, actor_name, action, resource_type, resource_id, remote_addr, details)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, created_at",
    )
    .bind(actor_admin_id)
    .bind(actor_name)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(remote_addr)
    .bind(details)
    .fetch_one(&mut *tx)
    .await?;
    let hash = auth::audit_chain_hash(
        prev.as_deref(),
        id,
        actor_name,
        action,
        resource_type,
        resource_id,
        &details_string,
        created.timestamp_micros(),
    );
    sqlx::query("UPDATE audit_events SET entry_hash = $2 WHERE id = $1")
        .bind(id)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Rows for chain verification (oldest first), including the stored hash.
#[allow(clippy::type_complexity)]
pub async fn audit_chain_rows(
    pool: &PgPool,
) -> Result<
    Vec<(
        i64,
        Option<String>,
        String,
        String,
        Option<String>,
        Json,
        DateTime<Utc>,
        Option<Vec<u8>>,
    )>,
> {
    Ok(sqlx::query_as(
        "SELECT id, actor_name, action, resource_type, resource_id, details, created_at, entry_hash
         FROM audit_events ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn list_audit_events(pool: &PgPool, limit: i64) -> Result<Vec<AuditEvent>> {
    Ok(
        sqlx::query_as("SELECT * FROM audit_events ORDER BY created_at DESC LIMIT $1")
            .bind(limit.clamp(1, 500))
            .fetch_all(pool)
            .await?,
    )
}

pub async fn start_node_push(
    pool: &PgPool,
    node_id: Uuid,
    desired_hash: &str,
    source: &str,
    actor_admin_id: Option<Uuid>,
) -> Result<i64> {
    let mut tx = pool.begin().await?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO node_push_events
           (node_id, desired_hash, source, status, actor_admin_id)
         VALUES ($1, $2, $3, 'started', $4) RETURNING id",
    )
    .bind(node_id)
    .bind(desired_hash)
    .bind(source)
    .bind(actor_admin_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE nodes SET desired_spec_hash = $2, last_push_at = now(),
           last_push_status = 'started', last_push_message = NULL WHERE id = $1",
    )
    .bind(node_id)
    .bind(desired_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn finish_node_push(
    pool: &PgPool,
    event_id: i64,
    node_id: Uuid,
    desired_hash: &str,
    status: &str,
    message: Option<&str>,
    applied_summary: Option<&Json>,
) -> Result<()> {
    let applied_hash = if status == "applied" || status == "unchanged" {
        Some(desired_hash)
    } else {
        None
    };
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE node_push_events SET status = $2, message = $3,
           applied_hash = $4, finished_at = now() WHERE id = $1",
    )
    .bind(event_id)
    .bind(status)
    .bind(message)
    .bind(applied_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE nodes SET last_push_at = now(), last_push_status = $3,
           last_push_message = $4,
           applied_spec_hash = COALESCE($2, applied_spec_hash),
           applied_spec_summary = CASE WHEN $2 IS NOT NULL THEN $5 ELSE applied_spec_summary END,
           applied_at = CASE WHEN $2 IS NOT NULL THEN now() ELSE applied_at END
         WHERE id = $1",
    )
    .bind(node_id)
    .bind(applied_hash)
    .bind(status)
    .bind(message)
    .bind(applied_summary)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn list_node_pushes(
    pool: &PgPool,
    node_id: Uuid,
    limit: i64,
) -> Result<Vec<NodePushEvent>> {
    Ok(sqlx::query_as(
        "SELECT * FROM node_push_events WHERE node_id = $1
         ORDER BY started_at DESC LIMIT $2",
    )
    .bind(node_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

pub async fn create_enrollment_token(
    pool: &PgPool,
    node_id: Uuid,
    token_hash: &[u8],
    created_by: Option<Uuid>,
    expires_at: DateTime<Utc>,
) -> Result<EnrollmentToken> {
    Ok(sqlx::query_as(
        "INSERT INTO node_enrollment_tokens
           (node_id, token_hash, created_by, expires_at)
         VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(node_id)
    .bind(token_hash)
    .bind(created_by)
    .bind(expires_at)
    .fetch_one(pool)
    .await?)
}

pub async fn list_enrollment_tokens(pool: &PgPool, node_id: Uuid) -> Result<Vec<EnrollmentToken>> {
    Ok(sqlx::query_as(
        "SELECT * FROM node_enrollment_tokens WHERE node_id = $1
         ORDER BY created_at DESC",
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?)
}

pub async fn revoke_enrollment_token(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query(
        "UPDATE node_enrollment_tokens SET revoked_at = now()
         WHERE id = $1 AND claimed_at IS NULL AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

pub async fn claim_enrollment_token(pool: &PgPool, token_hash: &[u8]) -> Result<Option<Uuid>> {
    Ok(sqlx::query_as::<_, (Uuid,)>(
        "UPDATE node_enrollment_tokens SET claimed_at = now()
         WHERE token_hash = $1 AND claimed_at IS NULL AND revoked_at IS NULL
           AND expires_at > now()
         RETURNING node_id",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?
    .map(|row| row.0))
}

pub async fn list_node_certificates(pool: &PgPool, node_id: Uuid) -> Result<Vec<NodeCertificate>> {
    Ok(
        sqlx::query_as(
            "SELECT * FROM node_certificates WHERE node_id = $1 ORDER BY issued_at DESC",
        )
        .bind(node_id)
        .fetch_all(pool)
        .await?,
    )
}

/// Fleet-wide enrollment inventory for the read-only health cockpit.
pub async fn list_node_certificates_all(pool: &PgPool) -> Result<Vec<NodeCertificate>> {
    Ok(
        sqlx::query_as("SELECT * FROM node_certificates ORDER BY node_id, issued_at DESC")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn add_node_certificate(
    pool: &PgPool,
    node_id: Uuid,
    serial_number: &str,
    fingerprint_sha256: &str,
    subject: &str,
    not_before: DateTime<Utc>,
    not_after: DateTime<Utc>,
) -> Result<NodeCertificate> {
    Ok(sqlx::query_as(
        "INSERT INTO node_certificates
           (node_id, serial_number, fingerprint_sha256, subject, not_before, not_after)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(node_id)
    .bind(serial_number)
    .bind(fingerprint_sha256)
    .bind(subject)
    .bind(not_before)
    .bind(not_after)
    .fetch_one(pool)
    .await?)
}

/// Return `(inventory_exists, presented_certificate_is_active)`. Legacy nodes
/// with no enrollment inventory remain CA-authenticated for compatibility;
/// once the first certificate is enrolled, the inventory becomes authoritative.
pub async fn authorize_node_certificate(
    pool: &PgPool,
    node_id: Uuid,
    fingerprint_sha256: &str,
) -> Result<(bool, bool)> {
    let (count, active): (i64, bool) = sqlx::query_as(
        "SELECT count(*), COALESCE(bool_or(
             upper(fingerprint_sha256) = upper($2)
             AND revoked_at IS NULL
             AND not_before <= now()
             AND not_after > now()
         ), false)
         FROM node_certificates WHERE node_id = $1",
    )
    .bind(node_id)
    .bind(fingerprint_sha256)
    .fetch_one(pool)
    .await?;
    Ok((count > 0, active))
}

pub async fn revoke_node_certificate(pool: &PgPool, certificate_id: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_as::<_, (Uuid,)>(
        "UPDATE node_certificates SET revoked_at = now()
         WHERE id = $1 AND revoked_at IS NULL RETURNING node_id",
    )
    .bind(certificate_id)
    .fetch_optional(pool)
    .await?
    .map(|row| row.0))
}

fn patch_parts<T>(patch: Patch<T>) -> (bool, Option<T>) {
    match patch {
        Patch::Missing => (false, None),
        Patch::Null => (true, None),
        Patch::Value(value) => (true, Some(value)),
    }
}

// --- app settings (runtime-editable key/value) ------------------------------

/// All stored settings as key→value pairs (missing keys fall back to defaults).
pub async fn all_settings(pool: &PgPool) -> Result<Vec<(String, String)>> {
    Ok(sqlx::query_as("SELECT key, value FROM app_settings")
        .fetch_all(pool)
        .await?)
}

pub async fn get_setting(pool: &PgPool, key: &str) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = $1")
            .bind(key)
            .fetch_optional(pool)
            .await?,
    )
}

/// A setting parsed as i64, falling back to `default` when unset or unparseable.
pub async fn setting_i64(pool: &PgPool, key: &str, default: i64) -> i64 {
    match get_setting(pool, key).await {
        Ok(Some(v)) => v.trim().parse().unwrap_or(default),
        _ => default,
    }
}

pub async fn set_setting(pool: &PgPool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES ($1, $2, now())
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

// --- scoped API keys --------------------------------------------------------

/// Create a key, returning the row plus the one-time plaintext token (only its
/// hash is stored).
pub async fn create_api_key(
    pool: &PgPool,
    name: &str,
    role: &str,
    created_by: Option<Uuid>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<(ApiKey, String)> {
    let token = format!("hny_{}", auth::random_token()?);
    let hash = auth::token_hash(&token);
    let key = sqlx::query_as(
        "INSERT INTO api_keys (name, key_hash, role, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5) RETURNING *",
    )
    .bind(name.trim())
    .bind(hash)
    .bind(role)
    .bind(created_by)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok((key, token))
}

pub async fn list_api_keys(pool: &PgPool) -> Result<Vec<ApiKey>> {
    Ok(
        sqlx::query_as("SELECT * FROM api_keys ORDER BY revoked_at IS NOT NULL, created_at DESC")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn revoke_api_key(pool: &PgPool, id: Uuid) -> Result<Option<ApiKey>> {
    Ok(sqlx::query_as(
        "UPDATE api_keys SET revoked_at = now()
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING *",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// Authenticate a live key and update its usage timestamp at most once per five
/// minutes. The CTE keeps lookup and the throttled touch in one database round
/// trip while avoiding a write for every API request.
pub async fn authenticate_api_key(pool: &PgPool, hash: &[u8]) -> Result<Option<ApiKey>> {
    Ok(sqlx::query_as(
        "WITH candidate AS MATERIALIZED (
           SELECT id FROM api_keys
           WHERE key_hash = $1 AND revoked_at IS NULL
             AND (expires_at IS NULL OR expires_at > now())
         ), touched AS (
           UPDATE api_keys AS k
           SET last_used_at = now()
           FROM candidate
           WHERE k.id = candidate.id
             AND (k.last_used_at IS NULL OR k.last_used_at < now() - interval '5 minutes')
           RETURNING k.*
         )
         SELECT * FROM touched
         UNION ALL
         SELECT k.* FROM api_keys AS k
         JOIN candidate ON candidate.id = k.id
         WHERE NOT EXISTS (SELECT 1 FROM touched)
         LIMIT 1",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?)
}

// --- scheduled operations ---------------------------------------------------

pub async fn create_scheduled_op(
    pool: &PgPool,
    input: &NewScheduledOp,
    created_by: Option<Uuid>,
) -> Result<ScheduledOp> {
    Ok(sqlx::query_as(
        "INSERT INTO scheduled_operations (resource_type, resource_id, action, run_at, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING *",
    )
    .bind(&input.resource_type)
    .bind(input.resource_id)
    .bind(&input.action)
    .bind(input.run_at)
    .bind(created_by)
    .fetch_one(pool)
    .await?)
}

pub async fn list_scheduled_ops(pool: &PgPool) -> Result<Vec<ScheduledOp>> {
    Ok(sqlx::query_as(
        "SELECT * FROM scheduled_operations
         ORDER BY status = 'pending' DESC, run_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn cancel_scheduled_op(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query(
        "UPDATE scheduled_operations SET status = 'canceled', updated_at = now()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

/// Due pending ops (claimed by the caller, which then executes & marks them).
pub async fn due_scheduled_ops(pool: &PgPool) -> Result<Vec<ScheduledOp>> {
    Ok(sqlx::query_as(
        "SELECT * FROM scheduled_operations
         WHERE status = 'pending' AND run_at <= now() ORDER BY run_at LIMIT 50",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn mark_scheduled_op(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    result: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE scheduled_operations SET status = $2, result = $3, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(result)
    .execute(pool)
    .await?;
    Ok(())
}

// --- entity change history (snapshots + revert) -----------------------------

/// Snapshot an entity, keeping only the most recent 20 versions per entity.
pub async fn record_entity_version(
    pool: &PgPool,
    resource_type: &str,
    resource_id: Uuid,
    snapshot: &serde_json::Value,
    actor: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO entity_versions (resource_type, resource_id, snapshot, actor)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(resource_type)
    .bind(resource_id)
    .bind(snapshot)
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM entity_versions WHERE resource_type = $1 AND resource_id = $2
           AND id NOT IN (
             SELECT id FROM entity_versions
             WHERE resource_type = $1 AND resource_id = $2
             ORDER BY id DESC LIMIT 20)",
    )
    .bind(resource_type)
    .bind(resource_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn list_entity_versions(
    pool: &PgPool,
    resource_type: &str,
    resource_id: Uuid,
) -> Result<Vec<EntityVersion>> {
    Ok(sqlx::query_as(
        "SELECT * FROM entity_versions WHERE resource_type = $1 AND resource_id = $2
         ORDER BY id DESC LIMIT 20",
    )
    .bind(resource_type)
    .bind(resource_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_entity_version(pool: &PgPool, id: i64) -> Result<Option<EntityVersion>> {
    Ok(
        sqlx::query_as("SELECT * FROM entity_versions WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Targeted enable/disable toggles used by the scheduler (no full-row update).
pub async fn set_node_enabled(pool: &PgPool, id: Uuid, enabled: bool) -> Result<bool> {
    Ok(sqlx::query("UPDATE nodes SET enabled = $2 WHERE id = $1")
        .bind(id)
        .bind(enabled)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

pub async fn set_user_enabled(pool: &PgPool, id: Uuid, enabled: bool) -> Result<bool> {
    Ok(sqlx::query("UPDATE users SET enabled = $2 WHERE id = $1")
        .bind(id)
        .bind(enabled)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

// --- custom RBAC roles ------------------------------------------------------

pub async fn list_custom_roles(pool: &PgPool) -> Result<Vec<CustomRole>> {
    Ok(sqlx::query_as("SELECT * FROM custom_roles ORDER BY name")
        .fetch_all(pool)
        .await?)
}

pub async fn create_custom_role(pool: &PgPool, input: &NewCustomRole) -> Result<CustomRole> {
    Ok(
        sqlx::query_as("INSERT INTO custom_roles (name, permissions) VALUES ($1, $2) RETURNING *")
            .bind(input.name.trim())
            .bind(&input.permissions)
            .fetch_one(pool)
            .await?,
    )
}

pub async fn update_custom_role(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateCustomRole,
) -> Result<Option<CustomRole>> {
    Ok(sqlx::query_as(
        "UPDATE custom_roles SET name = COALESCE($2, name),
           permissions = COALESCE($3, permissions), updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(input.name.as_deref().map(str::trim))
    .bind(input.permissions.as_ref())
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_custom_role(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM custom_roles WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// The permission matrix (domain -> level) for a role, parsed for the middleware.
pub async fn custom_role_permissions(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<std::collections::HashMap<String, i64>>> {
    let perms: Option<Json> =
        sqlx::query_scalar("SELECT permissions FROM custom_roles WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(perms.map(|v| serde_json::from_value(v).unwrap_or_default()))
}

/// Assign or clear (None) a custom role on an admin.
pub async fn set_admin_custom_role(
    pool: &PgPool,
    admin_id: Uuid,
    role_id: Option<Uuid>,
) -> Result<bool> {
    Ok(
        sqlx::query("UPDATE admins SET custom_role_id = $2 WHERE id = $1")
            .bind(admin_id)
            .bind(role_id)
            .execute(pool)
            .await?
            .rows_affected()
            > 0,
    )
}

// --- white-label branding ---------------------------------------------------

pub async fn get_branding(pool: &PgPool) -> Result<Branding> {
    // the singleton is seeded by migration; fall back to an INSERT if absent.
    if let Some(b) = sqlx::query_as::<_, Branding>("SELECT * FROM branding WHERE id = 1")
        .fetch_optional(pool)
        .await?
    {
        return Ok(b);
    }
    Ok(sqlx::query_as(
        "INSERT INTO branding (id) VALUES (1) ON CONFLICT (id) DO UPDATE SET id = 1 RETURNING *",
    )
    .fetch_one(pool)
    .await?)
}

pub async fn update_branding(pool: &PgPool, input: &UpdateBranding) -> Result<Branding> {
    Ok(sqlx::query_as(
        "UPDATE branding SET
           brand_name = COALESCE($1, brand_name),
           logo_url = COALESCE($2, logo_url),
           accent_color = COALESCE($3, accent_color),
           support_url = COALESCE($4, support_url),
           support_text = COALESCE($5, support_text),
           footer_text = COALESCE($6, footer_text),
           sub_welcome = COALESCE($7, sub_welcome),
           sub_show_imports = COALESCE($8, sub_show_imports),
           sub_show_downloads = COALESCE($9, sub_show_downloads),
           sub_show_endpoints = COALESCE($10, sub_show_endpoints),
           updated_at = now()
         WHERE id = 1 RETURNING *",
    )
    .bind(input.brand_name.as_deref().map(str::trim))
    .bind(input.logo_url.as_deref().map(str::trim))
    .bind(input.accent_color.as_deref().map(str::trim))
    .bind(input.support_url.as_deref().map(str::trim))
    .bind(input.support_text.as_deref().map(str::trim))
    .bind(input.footer_text.as_deref().map(str::trim))
    .bind(input.sub_welcome.as_deref().map(str::trim))
    .bind(input.sub_show_imports)
    .bind(input.sub_show_downloads)
    .bind(input.sub_show_endpoints)
    .fetch_one(pool)
    .await?)
}

// --- announcements ----------------------------------------------------------

pub async fn create_announcement(
    pool: &PgPool,
    input: &NewAnnouncement,
    created_by: Option<Uuid>,
) -> Result<Announcement> {
    Ok(sqlx::query_as(
        "INSERT INTO announcements (title, body, level, enabled, created_by)
         VALUES ($1, $2, $3, $4, $5) RETURNING *",
    )
    .bind(input.title.trim())
    .bind(input.body.trim())
    .bind(&input.level)
    .bind(input.enabled)
    .bind(created_by)
    .fetch_one(pool)
    .await?)
}

pub async fn list_announcements(pool: &PgPool) -> Result<Vec<Announcement>> {
    Ok(
        sqlx::query_as("SELECT * FROM announcements ORDER BY created_at DESC LIMIT 100")
            .fetch_all(pool)
            .await?,
    )
}

/// The most recent enabled announcement (shown publicly), if any.
pub async fn active_announcement(pool: &PgPool) -> Result<Option<Announcement>> {
    Ok(
        sqlx::query_as(
            "SELECT * FROM announcements WHERE enabled ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?,
    )
}

pub async fn update_announcement(
    pool: &PgPool,
    id: Uuid,
    input: &UpdateAnnouncement,
) -> Result<Option<Announcement>> {
    Ok(sqlx::query_as(
        "UPDATE announcements SET
           title = COALESCE($2, title), body = COALESCE($3, body),
           level = COALESCE($4, level), enabled = COALESCE($5, enabled), updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(input.title.as_deref().map(str::trim))
    .bind(input.body.as_deref().map(str::trim))
    .bind(input.level.as_deref())
    .bind(input.enabled)
    .fetch_optional(pool)
    .await?)
}

pub async fn delete_announcement(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM announcements WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// Enable/disable an inbound; returns its node id (for a re-push) if it existed.
pub async fn set_inbound_enabled(pool: &PgPool, id: Uuid, enabled: bool) -> Result<Option<Uuid>> {
    Ok(
        sqlx::query_scalar("UPDATE inbounds SET enabled = $2 WHERE id = $1 RETURNING node_id")
            .bind(id)
            .bind(enabled)
            .fetch_optional(pool)
            .await?,
    )
}

#[cfg(test)]
mod traffic_tests {
    use super::traffic_delta;

    #[test]
    fn traffic_delta_is_restart_safe() {
        assert_eq!(traffic_delta(None, "epoch-a", 100, 200), (0, 0));
        assert_eq!(
            traffic_delta(Some((100, 200, "epoch-a".into())), "epoch-a", 160, 260),
            (60, 60)
        );
        assert_eq!(
            traffic_delta(Some((160, 260, "epoch-a".into())), "epoch-b", 10, 20),
            (0, 0)
        );
    }

    #[test]
    fn traffic_delta_ignores_counter_regressions() {
        assert_eq!(
            traffic_delta(Some((160, 260, "epoch-a".into())), "epoch-a", 10, 20),
            (0, 0)
        );
        assert_eq!(
            traffic_delta(Some((160, 260, "epoch-a".into())), "epoch-a", 180, 20),
            (20, 0)
        );
    }
}
