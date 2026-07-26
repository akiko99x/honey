//! REST API: strict admin routes plus token-authenticated public subscriptions.
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Extension, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{self, Identity};
use crate::db::models::{
    Admin, AdminIp, AdminLoginEvent, AdminSession, ApiKey, AuditEvent, EnrollmentToken,
    FleetHealthSummary, GroupIds, Inbound, ManagedDomain, NewInbound, NewManagedDomain, NewNode,
    NewNodeGroup, NewNotifyChannel, NewRoutingProfile, NewSavedView, NewUser, Node,
    NodeCertificate, NodeGroup, NodePushEvent, NotifyChannel, OnboardingSnapshot, Patch,
    RotateCredentials, RoutingProfile, SavedView, SetLabels, SystemNotificationView, TelegramChat,
    TrafficCoreBreakdown, TrafficRank, TrafficSeriesPoint, UpdateInbound, UpdateManagedDomain,
    UpdateNode, UpdateNodeGroup, UpdateNotifyChannel, UpdateRoutingProfile, UpdateSavedView,
    UpdateUser, User,
};
use crate::db::repo;
use crate::registry::Registry;
use crate::secret;
use crate::subscription::{self, EndpointLink};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub registry: Arc<Registry>,
    pub certs_dir: PathBuf,
    pub api_token: Option<String>,
    pub login_limiter: Arc<crate::ratelimit::LoginLimiter>,
    pub subscription_limiter: Arc<crate::ratelimit::SubscriptionLimiter>,
}

pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/openapi.json", get(openapi_spec))
        .route("/announcement", get(public_announcement))
        .route("/branding", get(public_branding))
        .route("/status", get(status_page))
        .route("/auth/login", post(login))
        .route(
            "/sub-assets/subscription.css",
            get(crate::subscription_page::css),
        )
        .route(
            "/sub-assets/subscription.js",
            get(crate::subscription_page::js),
        )
        .route(
            "/sub-assets/PretendardVariable.woff2",
            get(crate::subscription_page::font),
        )
        .route("/enroll/:token/claim", post(claim_enrollment));

    let subscriptions = Router::new()
        .route("/sub/:token", get(subscription_document))
        .route("/sub/:token/links", get(subscription_links))
        .route("/sub/:token/v2ray", get(subscription_v2ray))
        .route("/sub/:token/sing-box", get(subscription_singbox))
        .route("/sub/:token/sing-box-tun", get(subscription_singbox_tun))
        .route("/sub/:token/clash", get(subscription_clash))
        .route("/sub/:token/qr", get(subscription_qr_all))
        .route("/sub/:token/qr/:inbound_id", get(subscription_qr))
        .route("/sub/:token/speedtest", get(subscription_speedtest))
        .route("/sub/:token/services", get(subscription_services))
        .route("/sub/:token/wireguard", get(subscription_wg_list))
        .route(
            "/sub/:token/wireguard/:iface_id",
            get(subscription_wg_config),
        )
        .route(
            "/sub/:token/wireguard/:iface_id/qr",
            get(subscription_wg_qr),
        )
        .route("/s/:alias", get(subscription_by_alias))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_subscription_access,
        ));

    let protected = Router::new()
        .route("/auth/me", get(me))
        .route("/auth/logout", post(logout))
        .route("/auth/sessions", get(list_sessions))
        .route("/auth/sessions/revoke-others", post(revoke_other_sessions))
        .route("/auth/sessions/:id", axum::routing::delete(revoke_session))
        .route("/auth/login-history", get(list_login_history))
        .route("/admins", get(list_admins).post(create_admin))
        .route("/admins/:id", axum::routing::patch(update_admin))
        .route("/admins/:id/groups", get(get_reseller_groups))
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/:id", axum::routing::delete(revoke_api_key))
        .route(
            "/custom-roles",
            get(list_custom_roles).post(create_custom_role),
        )
        .route(
            "/custom-roles/:id",
            axum::routing::patch(update_custom_role).delete(delete_custom_role),
        )
        .route("/import/users", post(import_users))
        .route("/config/export", get(config_export))
        .route("/config/apply", post(config_apply))
        .route("/auth/totp", get(totp_status))
        .route("/auth/totp/setup", post(totp_setup))
        .route("/auth/totp/enable", post(totp_enable))
        .route("/auth/totp/disable", post(totp_disable))
        .route("/auth/totp/recovery", get(totp_recovery_status))
        .route("/auth/totp/recovery/generate", post(totp_recovery_generate))
        .route("/admin-ips", get(list_admin_ips_h).post(add_admin_ip_h))
        .route("/admin-ips/:id", axum::routing::delete(delete_admin_ip_h))
        .route(
            "/users/:id/quota-interval",
            axum::routing::put(set_quota_interval),
        )
        .route("/audit", get(list_audit))
        .route("/audit/verify", get(verify_audit_chain))
        .route("/analytics/traffic", get(traffic_analytics))
        .route("/analytics/traffic.csv", get(traffic_analytics_csv))
        .route("/reports/period", get(period_report))
        .route("/analytics/geo", get(geo_distribution))
        .route("/ha", get(ha_status))
        .route("/update", get(update_check))
        .route("/update/apply", post(update_apply))
        .route("/settings", get(get_settings).patch(update_settings))
        .route("/system/logs", get(system_logs))
        .route("/issues", get(list_issues))
        .route("/onboarding", get(onboarding))
        .route("/notifications", get(list_notifications))
        .route(
            "/notifications/unread-count",
            get(notification_unread_count),
        )
        .route("/notifications/read-all", post(mark_all_notifications_read))
        .route("/notifications/:id/read", post(mark_notification_read))
        .route("/labels", get(list_labels))
        .route(
            "/saved-views",
            get(list_saved_views).post(create_saved_view),
        )
        .route(
            "/saved-views/:id",
            axum::routing::patch(update_saved_view).delete(delete_saved_view),
        )
        .route("/metrics", get(metrics))
        .route("/domains", get(list_domains).post(create_domain))
        .route(
            "/domains/:id",
            get(get_domain).patch(update_domain).delete(delete_domain),
        )
        .route("/domains/:id/verify", post(verify_domain))
        .route("/routing-profiles", get(list_profiles).post(create_profile))
        .route(
            "/routing-profiles/:id",
            axum::routing::patch(update_profile).delete(delete_profile),
        )
        .route(
            "/users/:id/routing-profile",
            axum::routing::put(assign_profile),
        )
        .route("/notify-channels", get(list_channels).post(create_channel))
        .route(
            "/notify-channels/:id",
            axum::routing::patch(update_channel).delete(delete_channel),
        )
        .route("/notify-channels/:id/test", post(test_channel))
        .route("/telegram-chats", get(list_tg_chats).post(add_tg_chat))
        .route(
            "/telegram-chats/:chat_id",
            axum::routing::delete(delete_tg_chat),
        )
        .route("/live-connections", get(live_connections))
        .route("/nodes", get(list_nodes).post(create_node))
        .route(
            "/nodes/:id",
            get(get_node).patch(update_node).delete(delete_node),
        )
        .route("/nodes/:id/inbounds", get(node_inbounds))
        .route("/nodes/:id/labels", axum::routing::put(set_node_labels))
        .route("/nodes/:id/config-preview", get(node_config_preview))
        .route("/nodes/:id/config-drift", get(node_config_drift))
        .route("/nodes/:id/preflight", get(node_preflight))
        .route("/nodes/:id/benchmark", post(node_benchmark))
        .route("/nodes/:id/dry-run", post(dry_run_node))
        .route("/nodes/:id/push", post(push_node))
        .route("/nodes/:id/pushes", get(node_pushes))
        .route("/nodes/:id/logs", get(node_agent_logs))
        .route("/nodes/:id/metrics", get(node_metrics))
        .route(
            "/nodes/:id/enrollments",
            get(node_enrollments).post(create_enrollment),
        )
        .route("/nodes/:id/certificates", get(node_certificates))
        .route("/nodes/:id/history", get(node_history))
        .route("/nodes/:id/revert/:version", post(revert_node))
        .route("/certificates/:id/revoke", post(revoke_certificate))
        .route("/branding", axum::routing::patch(update_branding_setting))
        .route(
            "/announcements",
            get(list_announcements).post(create_announcement),
        )
        .route(
            "/announcements/:id",
            axum::routing::patch(update_announcement).delete(delete_announcement),
        )
        .route(
            "/scheduled-ops",
            get(list_scheduled_ops).post(create_scheduled_op),
        )
        .route(
            "/scheduled-ops/:id",
            axum::routing::delete(cancel_scheduled_op),
        )
        .route("/enrollments/:id/revoke", post(revoke_enrollment))
        .route("/reality/keygen", post(reality_keygen))
        .route("/inbounds", post(create_inbound))
        .route("/nodes/:id/wireguard", get(list_wg).post(create_wg))
        .route(
            "/wireguard/:id",
            axum::routing::patch(update_wg).delete(delete_wg),
        )
        .route(
            "/nodes/:id/services",
            get(list_services).post(create_service),
        )
        .route(
            "/services/:id",
            axum::routing::patch(update_service).delete(delete_service),
        )
        .route(
            "/inbounds/:id/labels",
            axum::routing::put(set_inbound_labels),
        )
        .route(
            "/inbounds/:id",
            get(get_inbound)
                .patch(update_inbound)
                .delete(delete_inbound),
        )
        .route("/inbounds/:id/history", get(inbound_history))
        .route("/inbounds/:id/revert/:version", post(revert_inbound))
        .route("/users/:id/history", get(user_history))
        .route("/users/:id/revert/:version", post(revert_user))
        .route("/groups", get(list_groups).post(create_group))
        .route(
            "/groups/:id",
            axum::routing::patch(update_group).delete(delete_group),
        )
        .route(
            "/nodes/:id/groups",
            get(get_node_groups).put(set_node_groups),
        )
        .route(
            "/users/:id/groups",
            get(get_user_groups).put(set_user_groups),
        )
        .route("/inbounds/:id/reach", post(probe_inbound))
        .route(
            "/inbounds/:id/reachability",
            get(list_reachability).put(report_reachability),
        )
        .route("/inbounds/:id/rotate-sni", post(rotate_sni))
        .route("/users", get(list_users).post(create_user))
        .route("/users/:id/labels", axum::routing::put(set_user_labels))
        .route(
            "/users/:id",
            get(get_user).patch(update_user).delete(delete_user),
        )
        .route("/users/:id/rotate", post(rotate_user_credentials))
        .route("/users/:id/rotate-sub", post(rotate_subscription))
        .route("/users/:id/subscription", get(reveal_subscription))
        .route(
            "/users/:id/subscriptions",
            get(list_user_subscriptions).post(create_user_subscription),
        )
        .route(
            "/users/:id/subscriptions/:sid",
            get(reveal_user_subscription).delete(delete_user_subscription),
        )
        .route("/users/:id/alias", axum::routing::put(set_alias))
        .route("/users/:id/gdpr-export", get(gdpr_export))
        .route("/users/:id/gdpr-erase", post(gdpr_erase))
        .route("/users/:id/reset-traffic", post(reset_user_traffic))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    public
        .merge(subscriptions)
        .merge(protected)
        .fallback(crate::panel::serve)
        .layer(middleware::from_fn(request_id))
        .with_state(state)
}

fn apply_subscription_security_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store, max-age=0"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
}

fn subscription_path_identity(path: &str) -> Option<(&str, &str)> {
    let mut parts = path.trim_start_matches('/').split('/');
    match (parts.next(), parts.next()) {
        (Some("sub"), Some(value)) if !value.is_empty() => Some(("token", value)),
        (Some("s"), Some(value)) if !value.is_empty() => Some(("alias", value)),
        _ => None,
    }
}

async fn subscription_guard_subject(pool: &PgPool, path: &str) -> String {
    let Some((kind, value)) = subscription_path_identity(path) else {
        return "unknown".into();
    };
    let user_id = if kind == "token" {
        match value.parse::<Uuid>() {
            Ok(token) => repo::get_user_by_subscription_token(pool, token)
                .await
                .ok()
                .flatten()
                .map(|user| user.id),
            Err(_) => None,
        }
    } else {
        repo::get_user_by_alias(pool, value)
            .await
            .ok()
            .flatten()
            .map(|user| user.id)
    };
    user_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| format!("invalid:{}", auth::spec_hash(value.as_bytes())))
}

fn request_remote(req: &Request) -> String {
    forwarded_remote(req.headers())
        .filter(|value| value.parse::<std::net::IpAddr>().is_ok())
        .or_else(|| {
            req.extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|ConnectInfo(addr)| addr.ip().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

async fn require_subscription_access(
    State(st): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let config = subscription_guard_config(&st.pool).await;
    let subject = subscription_guard_subject(&st.pool, req.uri().path()).await;
    let client = request_remote(&req);
    let client_hash = auth::spec_hash(client.as_bytes());
    let subject_hash = auth::spec_hash(subject.as_bytes());
    let key = format!("{client_hash}:{subject_hash}");
    match st.subscription_limiter.check(&key, config).await {
        crate::ratelimit::LimitDecision::Allow => {
            let mut response = next.run(req).await;
            apply_subscription_security_headers(&mut response);
            response
        }
        crate::ratelimit::LimitDecision::Block { retry_after } => {
            tracing::warn!(
                code = "M1701",
                subscription = %subject_hash,
                client = %client_hash,
                retry_after,
                "public subscription request rate limited"
            );
            let pool = st.pool.clone();
            let dedupe = format!("subscription-abuse:{subject_hash}");
            let resource = subject_hash.clone();
            tokio::spawn(async move {
                crate::notify::alert(
                    &pool,
                    "subscription_abuse",
                    &dedupe,
                    "Subscription requests are being rate limited",
                    "A public subscription exceeded its configured request budget. Tokens and client addresses were not retained.",
                    &resource,
                )
                .await;
            });
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "error": "subscription request limit exceeded",
                    "code": "M1701",
                    "retry_after": retry_after
                })),
            )
                .into_response();
            if let Ok(value) = HeaderValue::from_str(&retry_after.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            apply_subscription_security_headers(&mut response);
            response
        }
    }
}

/// Generate or propagate an `x-request-id`, put it on a span so every log line
/// during the request (incl. push/reconcile/agent errors) carries it, and echo
/// it back in the response header for end-to-end correlation.
async fn request_id(req: Request, next: Next) -> Response {
    use tracing::Instrument;
    let id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let span = tracing::info_span!("http", request_id = %id);
    let id_for_header = id.clone();
    async move {
        let mut res = next.run(req).await;
        if let Ok(value) = HeaderValue::from_str(&id_for_header) {
            res.headers_mut().insert("x-request-id", value);
        }
        res
    }
    .instrument(span)
    .await
}

async fn require_auth(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let mut cookie_auth = false;
    let identity = if let Some(token) = provided {
        // 1. the legacy shared bearer token (back-compat) → owner scope.
        if st
            .api_token
            .as_deref()
            .is_some_and(|expected| ct_eq(token.as_bytes(), expected.as_bytes()))
        {
            Some(Identity::legacy())
        } else {
            // 2. a named, scoped API key (stored hashed).
            if !token.starts_with("hny_") {
                None
            } else {
                let hash = auth::token_hash(token);
                match repo::authenticate_api_key(&st.pool, &hash).await {
                    Ok(Some(key)) => Some(Identity {
                        admin_id: key.created_by,
                        username: format!("apikey:{}", key.name),
                        role: key.role,
                        session_hash: None,
                        permissions: None,
                    }),
                    Ok(None) => None,
                    Err(error) => {
                        tracing::error!(%error, "api key lookup failed");
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error": "internal server error", "code": "M1202"})),
                        )
                            .into_response();
                    }
                }
            }
        }
    } else if let Some(token) = cookie(req.headers(), "honey_session") {
        cookie_auth = true;
        let hash = auth::token_hash(token);
        match repo::admin_for_session(&st.pool, &hash).await {
            Ok(Some(admin)) => {
                let permissions = match admin.custom_role_id {
                    Some(crid) => repo::custom_role_permissions(&st.pool, crid)
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                };
                Some(Identity {
                    admin_id: Some(admin.id),
                    username: admin.username,
                    role: admin.role,
                    session_hash: Some(hash),
                    permissions,
                })
            }
            Ok(None) => None,
            Err(error) => {
                tracing::error!(%error, "session lookup failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal server error", "code": "M1202"})),
                )
                    .into_response();
            }
        }
    } else {
        None
    };
    let Some(identity) = identity else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized", "code": "M1207"})),
        )
            .into_response();
    };

    // admin IP allowlist (optional policy) — fail open on lookup error to avoid
    // a self-lockout if the db hiccups.
    let remote = forwarded_remote(req.headers());
    if !ip_allowed(&st.pool, remote.as_deref())
        .await
        .unwrap_or(true)
    {
        tracing::warn!(code = "M0305", user = %identity.username, ip = ?remote, "blocked by ip allowlist");
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "address not allowed", "code": "M1210"})),
        )
            .into_response();
    }

    if cookie_auth && !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        if req
            .headers()
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            == Some("cross-site")
        {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "cross-site request rejected", "code": "M1209"})),
            )
                .into_response();
        }
        if !origin_matches_host(req.headers()) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "origin does not match host", "code": "M1209"})),
            )
                .into_response();
        }
    }

    // resellers bypass the linear rank ladder entirely: they get a dedicated
    // scope allowlist here, and per-object ownership/entitlement is enforced
    // inside the individual handlers.
    if identity.is_reseller() {
        if !reseller_permits(req.method(), req.uri().path()) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "reseller scope does not allow this", "code": "M1211"})),
            )
                .into_response();
        }
    } else if identity.permissions.is_some() {
        // custom RBAC: a per-domain read/write matrix overrides the rank ladder.
        let domain = path_domain(req.uri().path());
        let need = if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
            1
        } else {
            2
        };
        if !identity.permits_domain(domain, need) {
            return (
                StatusCode::FORBIDDEN,
                Json(
                    json!({"error": format!("custom role lacks {domain} access"), "code": "M1210"}),
                ),
            )
                .into_response();
        }
    } else {
        let required = required_role(req.method(), req.uri().path());
        if !identity.permits(required) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": format!("{required} role required"), "code": "M1210"})),
            )
                .into_response();
        }
    }
    req.extensions_mut().insert(identity);
    next.run(req).await
}

/// A reseller may only touch its own users and read group names. Everything
/// else (nodes, inbounds, admins, settings, domains, rules…) is denied here;
/// per-user ownership and per-group entitlement are enforced in the handlers.
fn reseller_permits(method: &Method, path: &str) -> bool {
    match path {
        "/auth/me" => method == Method::GET,
        "/auth/logout" => method == Method::POST,
        "/auth/sessions" => method == Method::GET,
        "/auth/sessions/revoke-others" => method == Method::POST,
        "/auth/login-history" => method == Method::GET,
        "/groups" => method == Method::GET,
        "/labels" => method == Method::GET,
        "/analytics/traffic" => method == Method::GET,
        "/analytics/traffic.csv" => method == Method::GET,
        "/reports/period" => method == Method::GET,
        "/analytics/geo" => method == Method::GET,
        "/live-connections" => method == Method::GET,
        "/onboarding" => method == Method::GET,
        "/saved-views" => matches!(*method, Method::GET | Method::POST),
        "/users" => matches!(*method, Method::GET | Method::POST),
        _ => {
            if path.starts_with("/saved-views/") {
                return matches!(*method, Method::PATCH | Method::DELETE);
            }
            if path.starts_with("/auth/sessions/") {
                return method == Method::DELETE;
            }
            if let Some(rest) = path.strip_prefix("/users/") {
                // /users/:id, /users/:id/{rotate,reset-traffic,rotate-sub,groups}
                match rest.split_once('/') {
                    None => matches!(*method, Method::GET | Method::PATCH | Method::DELETE),
                    Some((_, "rotate")) => method == Method::POST,
                    Some((_, "reset-traffic")) => method == Method::POST,
                    Some((_, "rotate-sub")) => method == Method::POST,
                    Some((_, "subscription")) => method == Method::GET,
                    Some((_, "subscriptions")) => matches!(*method, Method::GET | Method::POST),
                    Some((_, sub)) if sub.starts_with("subscriptions/") => {
                        matches!(*method, Method::GET | Method::DELETE)
                    }
                    Some((_, "alias")) => method == Method::PUT,
                    Some((_, "groups")) => matches!(*method, Method::GET | Method::PUT),
                    Some((_, "labels")) => method == Method::PUT,
                    Some(_) => false,
                }
            } else {
                false
            }
        }
    }
}

/// Map a request path to a custom-RBAC domain. `dashboard` (always allowed) is
/// the fallback for read-only overview / personal surfaces.
fn path_domain(path: &str) -> &'static str {
    let p = path.trim_start_matches('/');
    if p.starts_with("nodes")
        || p.starts_with("certificates")
        || p.starts_with("enrollments")
        || p.starts_with("reality")
        || p.starts_with("wireguard")
        || p.starts_with("services")
    {
        "nodes"
    } else if p.starts_with("inbounds") {
        "inbounds"
    } else if p.starts_with("users") || p.starts_with("import") {
        "users"
    } else if p.starts_with("groups") {
        "groups"
    } else if p.starts_with("routing-profiles") {
        "routing"
    } else if p.starts_with("domains") {
        "domains"
    } else if p.starts_with("config") || p.starts_with("scheduled-ops") {
        "config"
    } else if p.starts_with("notify-channels")
        || p.starts_with("telegram")
        || p.starts_with("announcements")
    {
        "notifications"
    } else if p.starts_with("audit") {
        "audit"
    } else if p.starts_with("settings")
        || p.starts_with("branding")
        || p.starts_with("api-keys")
        || p.starts_with("admins")
        || p.starts_with("custom-roles")
        || p.starts_with("admin-ips")
    {
        "admin"
    } else {
        // issues, analytics, saved-views, onboarding, auth/me, metrics, …
        "dashboard"
    }
}

fn required_role(method: &Method, path: &str) -> &'static str {
    if path.starts_with("/notifications") {
        return "viewer";
    }
    if path.starts_with("/auth/sessions") || path == "/auth/login-history" {
        return "viewer";
    }
    if path.starts_with("/saved-views") {
        return "viewer";
    }
    if path.starts_with("/admins")
        || path.starts_with("/settings")
        || path.starts_with("/api-keys")
        || path.starts_with("/branding")
        || path.starts_with("/update")
        || path.starts_with("/config")
        || path.starts_with("/custom-roles")
    {
        return "owner";
    }
    if path.starts_with("/audit") || path.contains("/enrollments") {
        return "admin";
    }
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) || path == "/auth/logout" {
        return "viewer";
    }
    if path.ends_with("/push")
        || path.ends_with("/rotate")
        || path.ends_with("/rotate-sub")
        || path.ends_with("/reset-traffic")
        || path.ends_with("/labels")
    {
        return "operator";
    }
    "admin"
}

#[derive(Debug, Serialize, PartialEq)]
struct OnboardingStep {
    key: &'static str,
    label: &'static str,
    description: &'static str,
    complete: bool,
    route: &'static str,
    action: Option<&'static str>,
}

#[derive(Debug, Serialize, PartialEq)]
struct OnboardingView {
    completed: usize,
    total: usize,
    steps: Vec<OnboardingStep>,
}

fn build_onboarding(snapshot: OnboardingSnapshot, reseller: bool) -> OnboardingView {
    let mut steps = Vec::with_capacity(if reseller { 2 } else { 5 });
    if !reseller {
        steps.extend([
            OnboardingStep {
                key: "domain",
                label: "Register a domain",
                description: "Add an owned hostname for TLS or a public endpoint.",
                complete: snapshot.domain_count > 0,
                route: "domains",
                action: Some("add-domain"),
            },
            OnboardingStep {
                key: "node",
                label: "Connect a node",
                description: "Register a server and enroll its honey agent.",
                complete: snapshot.node_count > 0,
                route: "nodes",
                action: Some("add-node"),
            },
            OnboardingStep {
                key: "inbound",
                label: "Create an inbound",
                description: "Choose a core, protocol, security and transport.",
                complete: snapshot.inbound_count > 0,
                route: "inbounds",
                action: Some("add-inbound"),
            },
        ]);
    }
    steps.extend([
        OnboardingStep {
            key: "user",
            label: "Create a user",
            description: "Set access, quota and expiry for a subscriber.",
            complete: snapshot.user_count > 0,
            route: "users",
            action: Some("add-user"),
        },
        OnboardingStep {
            key: "subscription",
            label: "Share a subscription",
            description: "Reveal the current link and import it into a client.",
            complete: snapshot.subscription_count > 0,
            route: "subscriptions",
            action: None,
        },
    ]);
    let completed = steps.iter().filter(|step| step.complete).count();
    OnboardingView {
        completed,
        total: steps.len(),
        steps,
    }
}

async fn onboarding(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<OnboardingView>, ApiError> {
    let creator = if identity.is_reseller() {
        Some(
            identity
                .admin_id
                .ok_or_else(|| ApiError::forbidden("reseller identity has no account"))?,
        )
    } else {
        None
    };
    let snapshot = repo::onboarding_snapshot(&st.pool, creator).await?;
    Ok(Json(build_onboarding(snapshot, identity.is_reseller())))
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn origin_matches_host(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .is_some_and(|origin_host| origin_host.eq_ignore_ascii_case(host))
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

async fn root() -> &'static str {
    "honey master. who r u?\n"
}
async fn health() -> Json<JsonValue> {
    Json(json!({"status": "ok"}))
}

/// readiness: liveness (`/health`) says the process is up; this says the master
/// can actually reach its database.
async fn ready(State(st): State<AppState>) -> StatusCode {
    match sqlx::query("SELECT 1").execute(&st.pool).await {
        Ok(_) => StatusCode::OK,
        Err(error) => {
            tracing::warn!(code = "M0202", "readiness: db unreachable: {error}");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// Prometheus text exposition — no client library, just the format.
async fn metrics(State(st): State<AppState>) -> Result<Response, ApiError> {
    let (nodes_total, nodes_online, users_total, users_active, inbounds_total, traffic_total) =
        repo::metrics_snapshot(&st.pool).await?;
    let issue_counts = crate::issues::collect(&st.pool).await?.counts;
    let body = format!(
        "# HELP honey_nodes_total Registered nodes.\n\
         # TYPE honey_nodes_total gauge\n\
         honey_nodes_total {nodes_total}\n\
         # HELP honey_nodes_online Nodes seen in the last 2 minutes.\n\
         # TYPE honey_nodes_online gauge\n\
         honey_nodes_online {nodes_online}\n\
         # HELP honey_inbounds_total Configured inbounds.\n\
         # TYPE honey_inbounds_total gauge\n\
         honey_inbounds_total {inbounds_total}\n\
         # HELP honey_users_total Users.\n\
         # TYPE honey_users_total gauge\n\
         honey_users_total {users_total}\n\
         # HELP honey_users_active Active users (enabled, under quota, unexpired).\n\
         # TYPE honey_users_active gauge\n\
         honey_users_active {users_active}\n\
         # HELP honey_traffic_bytes_total Total used traffic across all users.\n\
         # TYPE honey_traffic_bytes_total counter\n\
         honey_traffic_bytes_total {traffic_total}\n\
         # HELP honey_issues Current health cockpit issues by severity.\n\
         # TYPE honey_issues gauge\n\
         honey_issues{{severity=\"critical\"}} {}\n\
         honey_issues{{severity=\"warning\"}} {}\n\
         honey_issues{{severity=\"info\"}} {}\n",
        issue_counts.critical, issue_counts.warning, issue_counts.info,
    );
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.11")], body).into_response())
}

async fn list_issues(
    State(st): State<AppState>,
) -> Result<Json<crate::issues::IssuesResponse>, ApiError> {
    Ok(Json(crate::issues::collect(&st.pool).await?))
}

#[derive(Deserialize)]
struct LabelQuery {
    resource: String,
}

fn normalize_labels(labels: Vec<String>) -> Result<Vec<String>, ApiError> {
    if labels.len() > 16 {
        return Err(ApiError::bad_request("at most 16 labels are allowed"));
    }
    let mut normalized = Vec::with_capacity(labels.len());
    for raw in labels {
        let label = raw.trim().to_ascii_lowercase();
        if label.is_empty()
            || label.len() > 40
            || !label.bytes().enumerate().all(|(index, byte)| match byte {
                b'a'..=b'z' | b'0'..=b'9' => true,
                b'.' | b'_' | b':' | b'-' => index > 0,
                _ => false,
            })
        {
            return Err(ApiError::bad_request(
                "labels must be 1-40 lowercase letters, digits, '.', '_', ':' or '-'",
            ));
        }
        normalized.push(label);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn saved_view_admin(identity: &Identity) -> Result<Uuid, ApiError> {
    identity
        .admin_id
        .ok_or_else(|| ApiError::forbidden("personal saved views require a panel session"))
}

fn ensure_view_resource(identity: &Identity, resource: &str) -> Result<(), ApiError> {
    if !matches!(resource, "nodes" | "inbounds" | "users" | "issues") {
        return Err(ApiError::bad_request("unsupported saved-view resource"));
    }
    if identity.is_reseller() && resource != "users" {
        return Err(ApiError::forbidden("resellers may only save user views"));
    }
    Ok(())
}

fn normalize_view_definition(resource: &str, definition: &mut JsonValue) -> Result<(), ApiError> {
    let object = definition
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("saved-view definition must be an object"))?;
    let allowed = [
        "search", "labels", "sort", "columns", "severity", "kind", "node",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ApiError::bad_request(
            "saved-view definition has an unknown field",
        ));
    }
    if let Some(search) = object.get_mut("search") {
        let value = search
            .as_str()
            .ok_or_else(|| ApiError::bad_request("saved-view search must be a string"))?
            .trim()
            .to_string();
        if value.len() > 200 {
            return Err(ApiError::bad_request("saved-view search is too long"));
        }
        *search = json!(value);
    }
    if let Some(labels) = object.get_mut("labels") {
        let values = labels
            .as_array()
            .ok_or_else(|| ApiError::bad_request("saved-view labels must be an array"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| ApiError::bad_request("saved-view labels must be strings"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        *labels = json!(normalize_labels(values)?);
    }
    let sorts: &[&str] = match resource {
        "nodes" => &["name", "status", "last_seen"],
        "inbounds" => &["tag", "node", "port"],
        "users" => &["username", "status", "traffic"],
        "issues" => &["severity", "detected", "entity"],
        _ => &[],
    };
    if let Some(sort) = object.get("sort") {
        if !sort.as_str().is_some_and(|value| sorts.contains(&value)) {
            return Err(ApiError::bad_request("unsupported saved-view sort"));
        }
    }
    let columns: &[&str] = match resource {
        "nodes" => &[
            "name",
            "address",
            "status",
            "labels",
            "transport",
            "version",
            "last_seen",
            "actions",
        ],
        "inbounds" => &[
            "tag", "node", "protocol", "labels", "core", "listen", "security", "status", "reach",
            "actions",
        ],
        "users" => &[
            "username", "uuid", "status", "labels", "traffic", "expires", "actions",
        ],
        "issues" => &[
            "severity", "code", "issue", "type", "entity", "labels", "node", "detected", "actions",
        ],
        _ => &[],
    };
    if let Some(selected) = object.get("columns") {
        let selected = selected
            .as_array()
            .ok_or_else(|| ApiError::bad_request("saved-view columns must be an array"))?;
        if selected.is_empty()
            || selected.iter().any(|value| {
                !value
                    .as_str()
                    .is_some_and(|column| columns.contains(&column))
            })
        {
            return Err(ApiError::bad_request(
                "unsupported or empty saved-view columns",
            ));
        }
    }
    for key in ["severity", "kind", "node"] {
        if let Some(value) = object.get(key) {
            if resource != "issues" || !value.as_str().is_some_and(|value| value.len() <= 64) {
                return Err(ApiError::bad_request(
                    "issue-only saved-view filter is invalid",
                ));
            }
        }
    }
    if serde_json::to_vec(definition)
        .map_err(|_| ApiError::bad_request("saved-view definition is invalid"))?
        .len()
        > 8192
    {
        return Err(ApiError::bad_request("saved-view definition is too large"));
    }
    Ok(())
}

async fn list_labels(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<LabelQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    if !matches!(
        query.resource.as_str(),
        "nodes" | "inbounds" | "users" | "issues"
    ) {
        return Err(ApiError::bad_request("unsupported label resource"));
    }
    if identity.is_reseller() {
        if !matches!(query.resource.as_str(), "users" | "issues") {
            return Err(ApiError::forbidden("label resource is outside your scope"));
        }
        let admin_id = saved_view_admin(&identity)?;
        return Ok(Json(
            repo::list_user_labels_for_creator(&st.pool, admin_id).await?,
        ));
    }
    let mut labels = if query.resource == "issues" {
        let mut labels = repo::list_labels(&st.pool, "nodes").await?;
        labels.extend(repo::list_labels(&st.pool, "inbounds").await?);
        labels.extend(repo::list_labels(&st.pool, "users").await?);
        labels
    } else {
        repo::list_labels(&st.pool, &query.resource).await?
    };
    labels.sort();
    labels.dedup();
    Ok(Json(labels))
}

async fn list_saved_views(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<SavedView>>, ApiError> {
    Ok(Json(
        repo::list_saved_views(&st.pool, saved_view_admin(&identity)?).await?,
    ))
}

async fn create_saved_view(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(mut input): Json<NewSavedView>,
) -> Result<Json<SavedView>, ApiError> {
    let admin_id = saved_view_admin(&identity)?;
    ensure_view_resource(&identity, &input.resource)?;
    input.name = input.name.trim().to_string();
    if input.name.is_empty() || input.name.len() > 80 {
        return Err(ApiError::bad_request(
            "saved-view name must be 1-80 characters",
        ));
    }
    normalize_view_definition(&input.resource, &mut input.definition)?;
    Ok(Json(
        repo::create_saved_view(
            &st.pool,
            admin_id,
            &input.name,
            &input.resource,
            &input.definition,
        )
        .await?,
    ))
}

async fn update_saved_view(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(mut input): Json<UpdateSavedView>,
) -> Result<Json<SavedView>, ApiError> {
    let admin_id = saved_view_admin(&identity)?;
    let current = repo::list_saved_views(&st.pool, admin_id)
        .await?
        .into_iter()
        .find(|view| view.id == id)
        .ok_or_else(|| ApiError::not_found("saved view not found"))?;
    ensure_view_resource(&identity, &current.resource)?;
    if let Some(name) = input.name.as_mut() {
        *name = name.trim().to_string();
        if name.is_empty() || name.len() > 80 {
            return Err(ApiError::bad_request(
                "saved-view name must be 1-80 characters",
            ));
        }
    }
    if let Some(definition) = input.definition.as_mut() {
        normalize_view_definition(&current.resource, definition)?;
    }
    Ok(Json(
        repo::update_saved_view(
            &st.pool,
            id,
            admin_id,
            input.name.as_deref(),
            input.definition.as_ref(),
        )
        .await?
        .ok_or_else(|| ApiError::not_found("saved view not found"))?,
    ))
}

async fn delete_saved_view(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_saved_view(&st.pool, id, saved_view_admin(&identity)?).await? {
        return Err(ApiError::not_found("saved view not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// --- 2FA (TOTP) -------------------------------------------------------------

#[derive(Deserialize)]
struct TotpCode {
    #[serde(default)]
    code: String,
}

async fn totp_status(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<JsonValue>, ApiError> {
    let admin_id = identity
        .admin_id
        .ok_or_else(|| ApiError::unauthorized("session required"))?;
    let enabled = repo::get_admin(&st.pool, admin_id)
        .await?
        .map(|a| a.totp_enabled)
        .unwrap_or(false);
    Ok(Json(json!({"enabled": enabled})))
}

async fn totp_setup(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<JsonValue>, ApiError> {
    let admin_id = identity
        .admin_id
        .ok_or_else(|| ApiError::unauthorized("session required"))?;
    let secret = auth::generate_totp_secret();
    let encrypted = secret::encrypt(&secret).map_err(|_| ApiError::internal("encrypt failed"))?;
    repo::set_admin_totp_secret(&st.pool, admin_id, &encrypted).await?;
    let url = auth::totp_provisioning_url(&secret, &identity.username)
        .map_err(|_| ApiError::internal("totp url"))?;
    let qr = crate::subscription_page::qr_svg(&url).map_err(|_| ApiError::internal("qr render"))?;
    Ok(Json(
        json!({"secret": secret, "otpauth_url": url, "qr_svg": qr}),
    ))
}

async fn totp_enable(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<TotpCode>,
) -> Result<StatusCode, ApiError> {
    let admin_id = identity
        .admin_id
        .ok_or_else(|| ApiError::unauthorized("session required"))?;
    let secret = repo::get_admin_totp_secret(&st.pool, admin_id)
        .await?
        .and_then(|enc| secret::decrypt(&enc).ok())
        .ok_or_else(|| ApiError::bad_request("run two-factor setup first"))?;
    if !auth::verify_totp(&secret, input.code.trim()) {
        return Err(ApiError::bad_request("invalid code"));
    }
    repo::set_admin_totp_enabled(&st.pool, admin_id, true).await?;
    audit(&st, &identity, "enable", "totp", Some(admin_id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn totp_disable(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<TotpCode>,
) -> Result<StatusCode, ApiError> {
    let admin_id = identity
        .admin_id
        .ok_or_else(|| ApiError::unauthorized("session required"))?;
    if let Some(secret) = repo::get_admin_totp_secret(&st.pool, admin_id)
        .await?
        .and_then(|enc| secret::decrypt(&enc).ok())
    {
        if !auth::verify_totp(&secret, input.code.trim()) {
            return Err(ApiError::bad_request("invalid code"));
        }
    }
    repo::clear_admin_totp(&st.pool, admin_id).await?;
    audit(&st, &identity, "disable", "totp", Some(admin_id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn totp_recovery_status(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<JsonValue>, ApiError> {
    let admin_id = session_account(&identity)?;
    let enabled = repo::get_admin(&st.pool, admin_id)
        .await?
        .map(|admin| admin.totp_enabled)
        .unwrap_or(false);
    let remaining = repo::count_unused_admin_recovery_codes(&st.pool, admin_id).await?;
    Ok(Json(json!({"enabled": enabled, "remaining": remaining})))
}

async fn totp_recovery_generate(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<TotpCode>,
) -> Result<Json<JsonValue>, ApiError> {
    let admin_id = session_account(&identity)?;
    let admin = repo::get_admin(&st.pool, admin_id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("session required"))?;
    if !admin.totp_enabled {
        return Err(ApiError::bad_request(
            "enable two-factor before generating recovery codes",
        ));
    }
    let secret = repo::get_admin_totp_secret(&st.pool, admin_id)
        .await?
        .and_then(|enc| secret::decrypt(&enc).ok())
        .ok_or_else(|| ApiError::bad_request("two-factor secret is unavailable"))?;
    if !auth::verify_totp(&secret, input.code.trim()) {
        return Err(ApiError::bad_request("invalid code"));
    }
    let mut codes = Vec::with_capacity(10);
    let mut hashes = Vec::with_capacity(10);
    for _ in 0..10 {
        let code = auth::generate_recovery_code().map_err(|_| ApiError::internal("rng failed"))?;
        hashes.push(auth::token_hash(&code));
        codes.push(code);
    }
    repo::replace_admin_recovery_codes(&st.pool, admin_id, &hashes).await?;
    audit(
        &st,
        &identity,
        "generate",
        "totp_recovery",
        Some(admin_id),
        json!({"count": codes.len()}),
    )
    .await;
    Ok(Json(json!({"codes": codes, "remaining": hashes.len()})))
}

// --- admin IP allowlist -----------------------------------------------------

fn normalize_cidr(raw: &str) -> Option<String> {
    if let Ok(net) = raw.parse::<ipnet::IpNet>() {
        return Some(net.to_string());
    }
    if let Ok(ip) = raw.parse::<std::net::IpAddr>() {
        let bits = if ip.is_ipv4() { 32 } else { 128 };
        return Some(format!("{ip}/{bits}"));
    }
    None
}

async fn list_admin_ips_h(State(st): State<AppState>) -> Result<Json<Vec<AdminIp>>, ApiError> {
    Ok(Json(repo::list_admin_ips(&st.pool).await?))
}

#[derive(Deserialize)]
struct NewAdminIp {
    cidr: String,
    #[serde(default)]
    note: String,
}

async fn add_admin_ip_h(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewAdminIp>,
) -> Result<Json<AdminIp>, ApiError> {
    let cidr = normalize_cidr(input.cidr.trim())
        .ok_or_else(|| ApiError::bad_request("cidr must be an IP or CIDR"))?;
    let entry = repo::add_admin_ip(&st.pool, &cidr, input.note.trim()).await?;
    audit(
        &st,
        &identity,
        "create",
        "admin_ip",
        Some(entry.id),
        json!({"cidr": cidr}),
    )
    .await;
    Ok(Json(entry))
}

async fn delete_admin_ip_h(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_admin_ip(&st.pool, id).await? {
        return Err(ApiError::not_found("entry not found"));
    }
    audit(&st, &identity, "delete", "admin_ip", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- periodic quota interval ------------------------------------------------

#[derive(Deserialize)]
struct QuotaInterval {
    interval: String,
}

async fn set_quota_interval(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<QuotaInterval>,
) -> Result<StatusCode, ApiError> {
    let interval = input.interval.trim();
    if !matches!(interval, "none" | "daily" | "weekly") {
        return Err(ApiError::bad_request(
            "interval must be none, daily or weekly",
        ));
    }
    let reset_at = if interval == "none" {
        None
    } else {
        Some(crate::quota::next_boundary(interval))
    };
    if !repo::set_user_quota_interval(&st.pool, id, interval, reset_at).await? {
        return Err(ApiError::not_found("user not found"));
    }
    audit(
        &st,
        &identity,
        "set_quota_interval",
        "user",
        Some(id),
        json!({"interval": interval}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct LoginInput {
    username: String,
    password: String,
    #[serde(default)]
    totp_code: Option<String>,
    #[serde(default)]
    recovery_code: Option<String>,
}

/// Returns false if an IP allowlist is configured and `remote` is not in it.
async fn ip_allowed(pool: &PgPool, remote: Option<&str>) -> Result<bool, ApiError> {
    let list = repo::list_admin_ips(pool).await?;
    if list.is_empty() {
        return Ok(true); // no policy configured
    }
    let Some(ip) = remote.and_then(|r| r.parse::<std::net::IpAddr>().ok()) else {
        return Ok(false); // policy on, but we can't tell the client IP
    };
    Ok(list.iter().any(|entry| {
        entry
            .cidr
            .parse::<ipnet::IpNet>()
            .map(|net| net.contains(&ip))
            .unwrap_or(false)
    }))
}

fn bounded_auth_text(value: &str, max: usize, fallback: &str) -> String {
    let value = value.trim();
    let value = if value.is_empty() { fallback } else { value };
    value.chars().take(max).collect()
}

async fn record_login_event(
    st: &AppState,
    admin_id: Option<Uuid>,
    username: &str,
    outcome: &str,
    remote_addr: Option<&str>,
    user_agent: Option<&str>,
) {
    let username = bounded_auth_text(username, 96, "<empty>");
    let remote_addr = remote_addr.map(|value| bounded_auth_text(value, 128, "unknown"));
    let user_agent = user_agent.map(|value| bounded_auth_text(value, 256, "unknown"));
    if let Err(error) = repo::record_admin_login_event(
        &st.pool,
        admin_id,
        &username,
        outcome,
        remote_addr.as_deref(),
        user_agent.as_deref(),
    )
    .await
    {
        tracing::error!(code = "M0306", %error, "could not persist login history");
    }
}

async fn login(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> Result<Response, ApiError> {
    let remote = forwarded_remote(&headers);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let limit_key = remote.clone().unwrap_or_else(|| "unknown".to_string());
    if let Some(retry_after) = st.login_limiter.locked_for(&limit_key).await {
        record_login_event(
            &st,
            None,
            &input.username,
            "rate_limited",
            remote.as_deref(),
            user_agent,
        )
        .await;
        tracing::warn!(code = "M0303", key = %limit_key, retry_after, "login locked out, too many tries");
        return Err(ApiError::too_many(format!(
            "too many login attempts; retry in {retry_after}s"
        )));
    }

    let candidate = repo::get_admin_by_username(&st.pool, &input.username)
        .await?
        .filter(|admin| admin.enabled);
    let candidate_id = candidate.as_ref().map(|admin| admin.id);
    let verified = match candidate {
        Some(admin) if auth::verify_password(&input.password, &admin.password_hash) => Some(admin),
        Some(_) => None,
        // No such admin (or a disabled one): still pay for one argon2
        // verification, or the faster reply would tell an attacker which
        // usernames are real.
        None => {
            auth::verify_dummy(&input.password);
            None
        }
    };
    let Some(admin) = verified else {
        st.login_limiter.record_failure(&limit_key).await;
        record_login_event(
            &st,
            candidate_id,
            &input.username,
            "bad_credentials",
            remote.as_deref(),
            user_agent,
        )
        .await;
        tracing::warn!(code = "M0302", user = %input.username, "bad login");
        return Err(ApiError::unauthorized("wrong username or password"));
    };

    // admin IP allowlist (optional policy).
    if !ip_allowed(&st.pool, remote.as_deref()).await? {
        st.login_limiter.record_failure(&limit_key).await;
        record_login_event(
            &st,
            Some(admin.id),
            &admin.username,
            "ip_denied",
            remote.as_deref(),
            user_agent,
        )
        .await;
        tracing::warn!(code = "M0305", user = %admin.username, ip = ?remote, "login from a disallowed address");
        return Err(ApiError::unauthorized("your address is not allowed"));
    }

    // second factor, if the admin has it enabled.
    if admin.totp_enabled {
        let totp_code = input.totp_code.as_deref().unwrap_or("").trim().to_string();
        let recovery_code = input
            .recovery_code
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();
        if totp_code.is_empty() && recovery_code.is_empty() {
            let remaining = repo::count_unused_admin_recovery_codes(&st.pool, admin.id).await?;
            return Ok((
                StatusCode::UNAUTHORIZED,
                Json(json!({"totp_required": true, "recovery_available": remaining > 0})),
            )
                .into_response());
        }
        let secret = repo::get_admin_totp_secret(&st.pool, admin.id)
            .await?
            .and_then(|enc| secret::decrypt(&enc).ok());
        let totp_ok = secret
            .as_deref()
            .map(|s| auth::verify_totp(s, &totp_code))
            .unwrap_or(false);
        let recovery_attempted = !recovery_code.is_empty();
        let recovery_ok = if !totp_ok && recovery_attempted {
            if let Some(code) = auth::normalize_recovery_code(&recovery_code) {
                repo::consume_admin_recovery_code(&st.pool, admin.id, &auth::token_hash(&code))
                    .await?
            } else {
                false
            }
        } else {
            false
        };
        if !totp_ok && !recovery_ok {
            st.login_limiter.record_failure(&limit_key).await;
            record_login_event(
                &st,
                Some(admin.id),
                &admin.username,
                if recovery_attempted {
                    "bad_recovery_code"
                } else {
                    "bad_totp"
                },
                remote.as_deref(),
                user_agent,
            )
            .await;
            tracing::warn!(code = "M0302", user = %admin.username, "bad two-factor code");
            return Err(ApiError::unauthorized(
                "invalid two-factor or recovery code",
            ));
        }
    }

    st.login_limiter.record_success(&limit_key).await;
    tracing::info!(code = "M0301", user = %admin.username, "signed in");

    let token = auth::random_token()?;
    let token_hash = auth::token_hash(&token);
    let expires_at = Utc::now() + Duration::hours(12);
    repo::create_admin_session(
        &st.pool,
        admin.id,
        &token_hash,
        expires_at,
        user_agent,
        remote.as_deref(),
    )
    .await?;
    record_login_event(
        &st,
        Some(admin.id),
        &admin.username,
        "success",
        remote.as_deref(),
        user_agent,
    )
    .await;
    repo::record_audit(
        &st.pool,
        Some(admin.id),
        Some(&admin.username),
        "login",
        "session",
        None,
        remote.as_deref(),
        json!({}),
    )
    .await?;
    let mut response = Json(json!({
        "admin": admin,
        "expires_at": expires_at
    }))
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&session_cookie(&token, &headers))
            .map_err(|_| ApiError::internal("could not create session cookie"))?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn me(Extension(identity): Extension<Identity>) -> Json<Identity> {
    Json(identity)
}

async fn logout(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Response, ApiError> {
    if let Some(hash) = identity.session_hash.as_deref() {
        repo::delete_admin_session(&st.pool, hash).await?;
    }
    tracing::info!(code = "M0304", user = %identity.username, "signed out");
    repo::record_audit(
        &st.pool,
        identity.admin_id,
        Some(&identity.username),
        "logout",
        "session",
        None,
        None,
        json!({}),
    )
    .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("honey_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    Ok(response)
}

#[derive(Deserialize)]
struct AdminSessionQuery {
    admin_id: Option<Uuid>,
}

#[derive(Serialize)]
struct AdminSessionView {
    #[serde(flatten)]
    session: AdminSession,
    current: bool,
}

fn session_account(identity: &Identity) -> Result<Uuid, ApiError> {
    identity
        .admin_id
        .ok_or_else(|| ApiError::forbidden("admin sessions require a panel session"))
}

fn may_manage_account(identity: &Identity, own_id: Uuid, target_id: Uuid) -> bool {
    target_id == own_id || (!identity.is_reseller() && identity.permits("admin"))
}

async fn list_sessions(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<AdminSessionQuery>,
) -> Result<Json<Vec<AdminSessionView>>, ApiError> {
    let own_id = session_account(&identity)?;
    let target_id = query.admin_id.unwrap_or(own_id);
    if !may_manage_account(&identity, own_id, target_id) {
        return Err(ApiError::forbidden("admin session is outside your scope"));
    }
    if repo::get_admin(&st.pool, target_id).await?.is_none() {
        return Err(ApiError::not_found("admin not found"));
    }
    let current_id = match identity.session_hash.as_deref() {
        Some(hash) => repo::admin_session_id_for_hash(&st.pool, hash).await?,
        None => None,
    };
    Ok(Json(
        repo::list_admin_sessions(&st.pool, target_id)
            .await?
            .into_iter()
            .map(|session| AdminSessionView {
                current: current_id == Some(session.id),
                session,
            })
            .collect(),
    ))
}

async fn revoke_session(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let own_id = session_account(&identity)?;
    let session = repo::get_admin_session(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("admin session not found"))?;
    if !may_manage_account(&identity, own_id, session.admin_id) {
        return Err(ApiError::forbidden("admin session is outside your scope"));
    }
    let current_id = match identity.session_hash.as_deref() {
        Some(hash) => repo::admin_session_id_for_hash(&st.pool, hash).await?,
        None => None,
    };
    let current = current_id == Some(id);
    if !repo::delete_admin_session_by_id(&st.pool, id).await? {
        return Err(ApiError::not_found("admin session not found"));
    }
    audit(
        &st,
        &identity,
        "revoke",
        "admin_session",
        Some(id),
        json!({"target_admin_id": session.admin_id, "current": current}),
    )
    .await;
    let mut response = StatusCode::NO_CONTENT.into_response();
    if current {
        response.headers_mut().insert(
            header::SET_COOKIE,
            HeaderValue::from_static(
                "honey_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0",
            ),
        );
    }
    Ok(response)
}

async fn revoke_other_sessions(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<JsonValue>, ApiError> {
    let admin_id = session_account(&identity)?;
    let current_hash = identity
        .session_hash
        .as_deref()
        .ok_or_else(|| ApiError::forbidden("current panel session is required"))?;
    let revoked = repo::delete_other_admin_sessions(&st.pool, admin_id, current_hash).await?;
    audit(
        &st,
        &identity,
        "revoke_others",
        "admin_session",
        None,
        json!({"revoked": revoked}),
    )
    .await;
    Ok(Json(json!({"revoked": revoked})))
}

#[derive(Deserialize)]
struct LoginHistoryQuery {
    admin_id: Option<Uuid>,
    #[serde(default)]
    all: bool,
    limit: Option<i64>,
}

async fn list_login_history(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<LoginHistoryQuery>,
) -> Result<Json<Vec<AdminLoginEvent>>, ApiError> {
    let own_id = session_account(&identity)?;
    let target_id = if query.all {
        if identity.is_reseller() || !identity.permits("admin") {
            return Err(ApiError::forbidden("all login history requires admin role"));
        }
        None
    } else {
        let target = query.admin_id.unwrap_or(own_id);
        if !may_manage_account(&identity, own_id, target) {
            return Err(ApiError::forbidden("login history is outside your scope"));
        }
        if repo::get_admin(&st.pool, target).await?.is_none() {
            return Err(ApiError::not_found("admin not found"));
        }
        Some(target)
    };
    Ok(Json(
        repo::list_admin_login_events(
            &st.pool,
            target_id,
            query.limit.unwrap_or(100).clamp(1, 200),
        )
        .await?,
    ))
}

fn session_cookie(token: &str, headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let forwarded_https = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("https"));
    let local =
        host.starts_with("127.0.0.1") || host.starts_with("localhost") || host.starts_with("[::1]");
    let secure = if forwarded_https || !local {
        "; Secure"
    } else {
        ""
    };
    format!("honey_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=43200{secure}")
}

fn forwarded_remote(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn list_admins(State(st): State<AppState>) -> Result<Json<Vec<Admin>>, ApiError> {
    Ok(Json(repo::list_admins(&st.pool).await?))
}

#[derive(Deserialize)]
struct CreateAdminInput {
    username: String,
    password: String,
    #[serde(default = "default_admin_role")]
    role: String,
    // reseller caps (0 = unlimited) and group entitlement; ignored otherwise.
    #[serde(default)]
    max_users: i32,
    #[serde(default)]
    user_traffic_ceiling_bytes: i64,
    #[serde(default)]
    traffic_limit_bytes: i64,
    #[serde(default)]
    commission_percent: i32,
    #[serde(default)]
    group_ids: Vec<Uuid>,
}

fn default_admin_role() -> String {
    "admin".into()
}

async fn create_admin(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<CreateAdminInput>,
) -> Result<Json<Admin>, ApiError> {
    if !auth::valid_role(&input.role) {
        return Err(ApiError::bad_request("invalid role"));
    }
    let is_reseller = input.role == "reseller";
    let password_hash = auth::hash_password(&input.password)
        .map_err(|error| ApiError::internal(format!("password hashing failed: {error}")))?;
    let admin = repo::create_admin(
        &st.pool,
        &input.username,
        &password_hash,
        &input.role,
        input.max_users.max(0),
        input.user_traffic_ceiling_bytes.max(0),
        input.traffic_limit_bytes.max(0),
        input.commission_percent.clamp(0, 100),
    )
    .await?;
    if is_reseller {
        repo::set_reseller_groups(&st.pool, admin.id, &input.group_ids).await?;
    }
    audit(
        &st,
        &identity,
        "create",
        "admin",
        Some(admin.id),
        json!({"role": admin.role}),
    )
    .await;
    Ok(Json(admin))
}

#[derive(Deserialize)]
struct UpdateAdminInput {
    role: Option<String>,
    enabled: Option<bool>,
    password: Option<String>,
    #[serde(default)]
    max_users: Option<i32>,
    #[serde(default)]
    user_traffic_ceiling_bytes: Option<i64>,
    #[serde(default)]
    traffic_limit_bytes: Option<i64>,
    #[serde(default)]
    commission_percent: Option<i32>,
    #[serde(default)]
    group_ids: Option<Vec<Uuid>>,
    /// custom RBAC role assignment; `Patch::Null` clears it back to the rank role.
    #[serde(default)]
    custom_role_id: Patch<Uuid>,
}

async fn update_admin(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateAdminInput>,
) -> Result<Json<Admin>, ApiError> {
    if input
        .role
        .as_deref()
        .is_some_and(|role| !auth::valid_role(role))
    {
        return Err(ApiError::bad_request("invalid role"));
    }
    if identity.admin_id == Some(id)
        && (input.enabled == Some(false)
            || input.role.as_deref().is_some_and(|role| role != "owner"))
    {
        return Err(ApiError::bad_request(
            "you cannot disable or demote your own owner account",
        ));
    }
    let password_hash = input
        .password
        .as_deref()
        .map(auth::hash_password)
        .transpose()
        .map_err(|error| ApiError::internal(format!("password hashing failed: {error}")))?;
    let admin = repo::update_admin(
        &st.pool,
        id,
        input.role.as_deref(),
        input.enabled,
        password_hash.as_deref(),
        input.max_users.map(|v| v.max(0)),
        input.user_traffic_ceiling_bytes.map(|v| v.max(0)),
        input.traffic_limit_bytes.map(|v| v.max(0)),
        input.commission_percent.map(|v| v.clamp(0, 100)),
    )
    .await?
    .ok_or_else(|| ApiError::not_found("admin not found"))?;
    if let Some(groups) = &input.group_ids {
        repo::set_reseller_groups(&st.pool, id, groups).await?;
    }
    match input.custom_role_id {
        Patch::Value(rid) => {
            repo::set_admin_custom_role(&st.pool, id, Some(rid)).await?;
        }
        Patch::Null => {
            repo::set_admin_custom_role(&st.pool, id, None).await?;
        }
        Patch::Missing => {}
    }
    if input.enabled == Some(false) || password_hash.is_some() {
        repo::delete_admin_sessions(&st.pool, id).await?;
    }
    audit(
        &st,
        &identity,
        "update",
        "admin",
        Some(id),
        json!({
            "role": input.role,
            "enabled": input.enabled,
            "password_changed": password_hash.is_some()
        }),
    )
    .await;
    Ok(Json(admin))
}

async fn get_reseller_groups(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Uuid>>, ApiError> {
    Ok(Json(repo::reseller_group_ids(&st.pool, id).await?))
}

// --- scoped API keys --------------------------------------------------------

#[derive(Serialize)]
struct ApiKeyView {
    #[serde(flatten)]
    key: ApiKey,
    status: &'static str,
}

fn api_key_status(key: &ApiKey, now: DateTime<Utc>) -> &'static str {
    if key.revoked_at.is_some() {
        "revoked"
    } else if key.expires_at.is_some_and(|expires| expires <= now) {
        "expired"
    } else {
        "active"
    }
}

fn api_key_view(key: ApiKey, now: DateTime<Utc>) -> ApiKeyView {
    let status = api_key_status(&key, now);
    ApiKeyView { key, status }
}

// --- custom RBAC roles ------------------------------------------------------

async fn list_custom_roles(
    State(st): State<AppState>,
) -> Result<Json<Vec<crate::db::models::CustomRole>>, ApiError> {
    Ok(Json(repo::list_custom_roles(&st.pool).await?))
}

/// Reject unknown domains / out-of-range levels so a role can't grant nonsense.
fn valid_permissions(perms: &JsonValue) -> bool {
    const DOMAINS: &[&str] = &[
        "nodes",
        "inbounds",
        "users",
        "groups",
        "routing",
        "domains",
        "config",
        "notifications",
        "audit",
        "admin",
    ];
    match perms.as_object() {
        Some(map) => map.iter().all(|(k, v)| {
            DOMAINS.contains(&k.as_str()) && v.as_i64().is_some_and(|n| (0..=2).contains(&n))
        }),
        None => false,
    }
}

async fn create_custom_role(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<crate::db::models::NewCustomRole>,
) -> Result<Json<crate::db::models::CustomRole>, ApiError> {
    if input.name.trim().is_empty() || input.name.len() > 64 {
        return Err(ApiError::bad_request("name is required (max 64 chars)"));
    }
    if !valid_permissions(&input.permissions) {
        return Err(ApiError::bad_request(
            "permissions must map known domains to 0/1/2",
        ));
    }
    let role = repo::create_custom_role(&st.pool, &input).await?;
    audit(
        &st,
        &identity,
        "create",
        "custom_role",
        Some(role.id),
        json!({"name": role.name}),
    )
    .await;
    Ok(Json(role))
}

async fn update_custom_role(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<crate::db::models::UpdateCustomRole>,
) -> Result<Json<crate::db::models::CustomRole>, ApiError> {
    if let Some(perms) = &input.permissions {
        if !valid_permissions(perms) {
            return Err(ApiError::bad_request(
                "permissions must map known domains to 0/1/2",
            ));
        }
    }
    let role = repo::update_custom_role(&st.pool, id, &input)
        .await?
        .ok_or_else(|| ApiError::not_found("custom role not found"))?;
    audit(&st, &identity, "update", "custom_role", Some(id), json!({})).await;
    Ok(Json(role))
}

async fn delete_custom_role(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_custom_role(&st.pool, id).await? {
        return Err(ApiError::not_found("custom role not found"));
    }
    audit(&st, &identity, "delete", "custom_role", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_api_keys(State(st): State<AppState>) -> Result<Json<Vec<ApiKeyView>>, ApiError> {
    let now = Utc::now();
    Ok(Json(
        repo::list_api_keys(&st.pool)
            .await?
            .into_iter()
            .map(|key| api_key_view(key, now))
            .collect(),
    ))
}

#[derive(Deserialize)]
struct NewApiKeyInput {
    name: String,
    #[serde(default = "default_key_role")]
    role: String,
    /// optional lifetime in days (0 / omitted = never expires).
    #[serde(default)]
    expires_days: i64,
}

fn default_key_role() -> String {
    "viewer".into()
}

#[derive(Serialize)]
struct CreatedApiKey {
    #[serde(flatten)]
    key: ApiKeyView,
    token: String,
}

fn validate_api_key_input(
    identity: &Identity,
    input: &NewApiKeyInput,
    now: DateTime<Utc>,
) -> Result<(String, String, Option<DateTime<Utc>>), ApiError> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "name must be 1-64 printable characters",
        ));
    }
    let role = input.role.trim();
    if !matches!(role, "owner" | "admin" | "operator" | "viewer") {
        return Err(ApiError::bad_request(
            "scope must be owner/admin/operator/viewer",
        ));
    }
    if identity.is_reseller() || !identity.permits(role) {
        return Err(ApiError::forbidden("cannot mint a key above your own role"));
    }
    if !(0..=3650).contains(&input.expires_days) {
        return Err(ApiError::bad_request(
            "expires_days must be between 0 and 3650",
        ));
    }
    let expires_at =
        (input.expires_days > 0).then(|| now + chrono::Duration::days(input.expires_days));
    Ok((name.into(), role.into(), expires_at))
}

async fn create_api_key(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewApiKeyInput>,
) -> Result<Json<CreatedApiKey>, ApiError> {
    let now = Utc::now();
    let (name, role, expires_at) = validate_api_key_input(&identity, &input, now)?;
    let (key, token) =
        repo::create_api_key(&st.pool, &name, &role, identity.admin_id, expires_at).await?;
    audit(
        &st,
        &identity,
        "create",
        "api_key",
        Some(key.id),
        json!({"name": key.name, "role": key.role, "expires_at": key.expires_at}),
    )
    .await;
    Ok(Json(CreatedApiKey {
        key: api_key_view(key, now),
        token,
    }))
}

async fn revoke_api_key(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let Some(key) = repo::revoke_api_key(&st.pool, id).await? else {
        return Err(ApiError::not_found("api key not found or already revoked"));
    };
    audit(
        &st,
        &identity,
        "revoke",
        "api_key",
        Some(id),
        json!({"name": key.name, "role": key.role}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Curated OpenAPI 3 description of the honey master API (bearer API-key auth).
async fn openapi_spec() -> Response {
    const SPEC: &str = include_str!("../../../web/openapi.json");
    let mut response = SPEC.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

// --- scheduled operations ---------------------------------------------------

async fn list_scheduled_ops(
    State(st): State<AppState>,
) -> Result<Json<Vec<crate::db::models::ScheduledOp>>, ApiError> {
    Ok(Json(repo::list_scheduled_ops(&st.pool).await?))
}

async fn create_scheduled_op(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<crate::db::models::NewScheduledOp>,
) -> Result<Json<crate::db::models::ScheduledOp>, ApiError> {
    let allowed: &[&str] = match input.resource_type.as_str() {
        "node" => &["enable", "disable", "push"],
        "user" => &["enable", "disable", "reset-traffic", "rotate-sub"],
        "inbound" => &["enable", "disable"],
        _ => {
            return Err(ApiError::bad_request(
                "resource_type must be node/user/inbound",
            ))
        }
    };
    if !allowed.contains(&input.action.as_str()) {
        return Err(ApiError::bad_request(
            "unsupported action for this resource type",
        ));
    }
    if input.run_at < Utc::now() - chrono::Duration::minutes(1) {
        return Err(ApiError::bad_request("run_at must be in the future"));
    }
    let op = repo::create_scheduled_op(&st.pool, &input, identity.admin_id).await?;
    audit(
        &st,
        &identity,
        "schedule",
        &input.resource_type,
        Some(input.resource_id),
        json!({"action": input.action, "run_at": input.run_at}),
    )
    .await;
    Ok(Json(op))
}

async fn cancel_scheduled_op(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::cancel_scheduled_op(&st.pool, id).await? {
        return Err(ApiError::not_found("scheduled op not found or not pending"));
    }
    audit(
        &st,
        &identity,
        "cancel",
        "scheduled_op",
        Some(id),
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// --- entity change history / revert -----------------------------------------

/// Snapshot an entity into its version history (best-effort).
async fn capture_version<T: Serialize>(
    st: &AppState,
    resource_type: &str,
    id: Uuid,
    entity: &T,
    identity: &Identity,
) {
    if let Ok(snapshot) = serde_json::to_value(entity) {
        if let Err(error) = repo::record_entity_version(
            &st.pool,
            resource_type,
            id,
            &snapshot,
            Some(&identity.username),
        )
        .await
        {
            tracing::warn!(%error, "could not record entity version");
        }
    }
}

async fn node_history(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<crate::db::models::EntityVersion>>, ApiError> {
    Ok(Json(
        repo::list_entity_versions(&st.pool, "node", id).await?,
    ))
}

async fn user_history(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<crate::db::models::EntityVersion>>, ApiError> {
    owned_user(&st, &identity, id).await?;
    Ok(Json(
        repo::list_entity_versions(&st.pool, "user", id).await?,
    ))
}

async fn inbound_history(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<crate::db::models::EntityVersion>>, ApiError> {
    Ok(Json(
        repo::list_entity_versions(&st.pool, "inbound", id).await?,
    ))
}

/// Fetch a version and confirm it belongs to the expected entity.
async fn load_version(
    st: &AppState,
    version: i64,
    resource_type: &str,
    id: Uuid,
) -> Result<JsonValue, ApiError> {
    let ver = repo::get_entity_version(&st.pool, version)
        .await?
        .filter(|v| v.resource_type == resource_type && v.resource_id == id)
        .ok_or_else(|| ApiError::not_found("version not found for this entity"))?;
    Ok(ver.snapshot)
}

async fn revert_node(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path((id, version)): Path<(Uuid, i64)>,
) -> Result<Json<Node>, ApiError> {
    let snapshot = load_version(&st, version, "node", id).await?;
    let update: UpdateNode = serde_json::from_value(snapshot)
        .map_err(|error| ApiError::bad_request(format!("snapshot is not revertable: {error}")))?;
    let node = repo::update_node(&st.pool, id, update)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    push_nodes(&st, [id]).await;
    capture_version(&st, "node", id, &node, &identity).await;
    audit(
        &st,
        &identity,
        "revert",
        "node",
        Some(id),
        json!({"version": version}),
    )
    .await;
    Ok(Json(node))
}

async fn revert_user(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path((id, version)): Path<(Uuid, i64)>,
) -> Result<Json<UserView>, ApiError> {
    owned_user(&st, &identity, id).await?;
    let snapshot = load_version(&st, version, "user", id).await?;
    let update: UpdateUser = serde_json::from_value(snapshot)
        .map_err(|error| ApiError::bad_request(format!("snapshot is not revertable: {error}")))?;
    let nodes = repo::user_node_ids(&st.pool, id).await?;
    let user = repo::update_user(&st.pool, id, update)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    push_nodes(&st, nodes).await;
    capture_version(&st, "user", id, &user, &identity).await;
    audit(
        &st,
        &identity,
        "revert",
        "user",
        Some(id),
        json!({"version": version}),
    )
    .await;
    Ok(Json(user.into()))
}

async fn revert_inbound(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path((id, version)): Path<(Uuid, i64)>,
) -> Result<Json<Inbound>, ApiError> {
    let snapshot = load_version(&st, version, "inbound", id).await?;
    let update: UpdateInbound = serde_json::from_value(snapshot)
        .map_err(|error| ApiError::bad_request(format!("snapshot is not revertable: {error}")))?;
    let inbound = repo::update_inbound(&st.pool, id, update)
        .await?
        .ok_or_else(|| ApiError::not_found("inbound not found"))?;
    push_nodes(&st, [inbound.node_id]).await;
    capture_version(&st, "inbound", id, &inbound, &identity).await;
    audit(
        &st,
        &identity,
        "revert",
        "inbound",
        Some(id),
        json!({"version": version}),
    )
    .await;
    Ok(Json(inbound))
}

// --- announcements + public status ------------------------------------------

async fn list_announcements(
    State(st): State<AppState>,
) -> Result<Json<Vec<crate::db::models::Announcement>>, ApiError> {
    Ok(Json(repo::list_announcements(&st.pool).await?))
}

fn valid_level(level: &str) -> bool {
    matches!(level, "info" | "warning" | "critical")
}

async fn create_announcement(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<crate::db::models::NewAnnouncement>,
) -> Result<Json<crate::db::models::Announcement>, ApiError> {
    if input.title.trim().is_empty() || input.title.len() > 200 {
        return Err(ApiError::bad_request("title is required (max 200 chars)"));
    }
    if !valid_level(&input.level) {
        return Err(ApiError::bad_request("level must be info/warning/critical"));
    }
    let ann = repo::create_announcement(&st.pool, &input, identity.admin_id).await?;
    audit(
        &st,
        &identity,
        "create",
        "announcement",
        Some(ann.id),
        json!({"title": ann.title}),
    )
    .await;
    Ok(Json(ann))
}

async fn update_announcement(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<crate::db::models::UpdateAnnouncement>,
) -> Result<Json<crate::db::models::Announcement>, ApiError> {
    if input.level.as_deref().is_some_and(|l| !valid_level(l)) {
        return Err(ApiError::bad_request("level must be info/warning/critical"));
    }
    let ann = repo::update_announcement(&st.pool, id, &input)
        .await?
        .ok_or_else(|| ApiError::not_found("announcement not found"))?;
    audit(
        &st,
        &identity,
        "update",
        "announcement",
        Some(id),
        json!({}),
    )
    .await;
    Ok(Json(ann))
}

async fn delete_announcement(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_announcement(&st.pool, id).await? {
        return Err(ApiError::not_found("announcement not found"));
    }
    audit(
        &st,
        &identity,
        "delete",
        "announcement",
        Some(id),
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Public: the active announcement (for the subscription-page banner). Always
/// 200 with a possibly-null body so the client can fetch unconditionally.
async fn public_announcement(State(st): State<AppState>) -> Response {
    match repo::active_announcement(&st.pool).await {
        Ok(Some(ann)) => Json(json!({
            "title": ann.title, "body": ann.body, "level": ann.level
        }))
        .into_response(),
        _ => Json(json!(null)).into_response(),
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn public_branding(
    State(st): State<AppState>,
) -> Result<Json<crate::db::models::Branding>, ApiError> {
    Ok(Json(repo::get_branding(&st.pool).await?))
}

/// A safe href: only http(s) or a same-origin path; blocks javascript:/data: URIs.
fn safe_url(url: &str) -> bool {
    url.is_empty()
        || url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with('/')
}

async fn update_branding_setting(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<crate::db::models::UpdateBranding>,
) -> Result<Json<crate::db::models::Branding>, ApiError> {
    if let Some(color) = input.accent_color.as_deref() {
        let color = color.trim();
        let ok = color.is_empty()
            || (color.len() == 7
                && color.starts_with('#')
                && color[1..].bytes().all(|b| b.is_ascii_hexdigit()));
        if !ok {
            return Err(ApiError::bad_request(
                "accent_color must be a #rrggbb hex value",
            ));
        }
    }
    for url in [input.logo_url.as_deref(), input.support_url.as_deref()] {
        if let Some(url) = url {
            if !safe_url(url.trim()) {
                return Err(ApiError::bad_request("urls must be http(s) or a / path"));
            }
        }
    }
    let branding = repo::update_branding(&st.pool, &input).await?;
    audit(&st, &identity, "update", "branding", None, json!({})).await;
    Ok(Json(branding))
}

/// Public status page: aggregate fleet health + the active announcement.
async fn status_page(State(st): State<AppState>) -> Response {
    let brand = repo::get_branding(&st.pool)
        .await
        .map(|b| b.brand_name)
        .unwrap_or_else(|_| "honey".to_string());
    let brand = html_escape(&brand);
    let nodes = repo::list_nodes(&st.pool).await.unwrap_or_default();
    let enabled: Vec<_> = nodes.iter().filter(|n| n.enabled).collect();
    let cutoff = Utc::now() - chrono::Duration::minutes(2);
    let is_online = |n: &&crate::db::models::Node| n.last_seen.is_some_and(|seen| seen > cutoff);
    let online = enabled.iter().filter(|n| is_online(n)).count();
    let total = enabled.len();
    let (state_class, state_word) = if total == 0 {
        ("warn", "no nodes")
    } else if online == total {
        ("ok", "operational")
    } else if online == 0 {
        ("bad", "major outage")
    } else {
        ("warn", "degraded")
    };

    // per-node availability rows (24h uptime %), addresses intentionally omitted.
    let uptime: std::collections::HashMap<uuid::Uuid, f64> = repo::node_uptime(&st.pool, 24)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|u| (u.node_id, u.ratio))
        .collect();
    let node_rows: String = enabled
        .iter()
        .map(|n| {
            let (cls, word) = if n.maintenance {
                ("warn", "maintenance")
            } else if is_online(n) {
                ("ok", "operational")
            } else {
                ("bad", "offline")
            };
            let up = match uptime.get(&n.id) {
                Some(r) => format!("{:.2}%", (r * 100.0).clamp(0.0, 100.0)),
                None => "—".to_string(),
            };
            format!(
                "<div class=\"row\"><span class=\"nodecell\"><span class=\"dot {cls}\"></span>{name}</span><span class=\"muted\">{up}<b class=\"{cls}\">{word}</b></span></div>",
                name = html_escape(&n.name),
            )
        })
        .collect();

    // recent availability incidents (last 7 days), newest first.
    let incidents = repo::recent_incidents(&st.pool, 7, 10)
        .await
        .unwrap_or_default();
    let incident_html = if incidents.is_empty() {
        String::new()
    } else {
        let rows: String = incidents
            .iter()
            .map(|i| {
                let sev = match i.severity.as_str() {
                    "critical" => "bad",
                    "warning" => "warn",
                    _ => "muted",
                };
                let repeats = if i.occurrence_count > 1 {
                    format!(" ×{}", i.occurrence_count)
                } else {
                    String::new()
                };
                format!(
                    "<div class=\"row\"><span class=\"nodecell\"><span class=\"dot {sev}\"></span>{title}{repeats}</span><span class=\"muted\">{when}</span></div>",
                    title = html_escape(&i.title),
                    when = i.last_seen_at.format("%d %b %H:%M"),
                )
            })
            .collect();
        format!("<div class=\"sect\">Recent incidents (7d)</div>{rows}")
    };
    let banner = match repo::active_announcement(&st.pool).await {
        Ok(Some(a)) => format!(
            "<div class=\"note {}\"><b>{}</b>{}</div>",
            html_escape(&a.level),
            html_escape(&a.title),
            if a.body.is_empty() {
                String::new()
            } else {
                format!("<p>{}</p>", html_escape(&a.body))
            }
        ),
        _ => String::new(),
    };
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{brand} status</title><style>\
body{{margin:0;background:#080808;color:#f5f5f5;font-family:system-ui,-apple-system,sans-serif;display:flex;min-height:100vh;align-items:center;justify-content:center}}\
.card{{width:min(92vw,520px);padding:34px;border:1px solid #292929;border-radius:14px;background:#0d0d0d}}\
h1{{font-size:19px;margin:0 0 4px}}.sub{{color:#a2a2a2;font-size:13px;margin-bottom:22px}}\
.big{{display:flex;align-items:center;gap:12px;font-size:22px;font-weight:650;margin-bottom:20px}}\
.dot{{width:13px;height:13px;border-radius:50%}}.ok .dot,.dot.ok{{background:#42c98a}}.warn .dot,.dot.warn{{background:#e2b84b}}.bad .dot,.dot.bad{{background:#e56b6b}}\
.row{{display:flex;justify-content:space-between;align-items:center;padding:11px 0;border-top:1px solid #1c1c1c;font-size:14px}}\
.muted{{color:#a2a2a2}}.note{{margin:0 0 18px;padding:12px 14px;border-radius:9px;border:1px solid #333;background:#141414}}\
.note.warning{{border-color:#7a6318}}.note.critical{{border-color:#7a2b2b}}.note b{{display:block;font-size:13px}}.note p{{margin:6px 0 0;color:#cfcfcf;font-size:12px}}\
.nodecell{{display:flex;align-items:center;gap:9px}}.nodecell .dot{{width:9px;height:9px}}\
.row .muted{{display:flex;align-items:center;gap:10px;font-size:12px}}.row .muted b{{font-weight:600}}\
b.ok{{color:#42c98a}}b.warn{{color:#e2b84b}}b.bad{{color:#e56b6b}}\
.sect{{margin:22px 0 2px;color:#7f7f7f;font-size:11px;text-transform:uppercase;letter-spacing:.08em}}\
</style></head><body><div class=\"card\">\
<h1>{brand} status</h1><div class=\"sub\">public availability of this deployment</div>\
{banner}\
<div class=\"big {state_class}\"><span class=\"dot {state_class}\"></span>{state_word}</div>\
<div class=\"row\"><span class=\"muted\">Nodes online</span><b>{online} / {total}</b></div>\
{node_rows}\
{incident_html}\
</div></body></html>"
    );
    crate::subscription_page::html_response(body)
}

// --- bulk import + config-as-code (GitOps) ----------------------------------

#[derive(Deserialize)]
struct ImportUser {
    username: String,
    #[serde(default)]
    traffic_limit_bytes: Option<i64>,
    // marzban-style aliases
    #[serde(default)]
    data_limit: Option<i64>,
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
    /// unix-seconds expiry (marzban `expire`); 0/absent = never.
    #[serde(default)]
    expire: Option<i64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct ImportUsersInput {
    users: Vec<ImportUser>,
}

/// Bulk-create users from another panel's export (generic or Marzban-shaped).
/// Existing usernames are skipped; each new user gets a fresh honey credential.
async fn import_users(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<ImportUsersInput>,
) -> Result<Json<JsonValue>, ApiError> {
    let mut created = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;
    let mut created_names: Vec<String> = Vec::new();
    for u in input.users {
        let name = u.username.trim().to_string();
        if name.is_empty() {
            failed += 1;
            continue;
        }
        if repo::get_user_by_name(&st.pool, &name)
            .await
            .ok()
            .flatten()
            .is_some()
        {
            skipped += 1;
            continue;
        }
        let expires_at = u.expires_at.or_else(|| {
            u.expire
                .filter(|s| *s > 0)
                .and_then(|s| DateTime::from_timestamp(s, 0))
        });
        let enabled = u
            .enabled
            .unwrap_or_else(|| !matches!(u.status.as_deref(), Some("disabled") | Some("expired")));
        let password = match auth::random_token() {
            Ok(token) => token,
            Err(_) => {
                failed += 1;
                continue;
            }
        };
        let new = NewUser {
            username: name.clone(),
            password,
            subscription_title: None,
            subscription_description: None,
            traffic_limit_bytes: u.traffic_limit_bytes.or(u.data_limit).unwrap_or(0).max(0),
            expires_at,
            device_limit: 0,
        };
        match repo::create_user(&st.pool, new, identity.admin_id, true).await {
            Ok((user, _)) => {
                if !enabled {
                    let _ = repo::set_user_enabled(&st.pool, user.id, false).await;
                }
                created += 1;
                created_names.push(name);
            }
            Err(_) => failed += 1,
        }
    }
    if !created_names.is_empty() {
        // new users may reach ungrouped nodes; push so their configs land.
        let nodes: std::collections::HashSet<Uuid> = repo::list_nodes(&st.pool)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|n| n.id)
            .collect();
        push_nodes(&st, nodes).await;
    }
    audit(
        &st,
        &identity,
        "import",
        "user",
        None,
        json!({"created": created, "skipped": skipped, "failed": failed}),
    )
    .await;
    Ok(Json(
        json!({"created": created, "skipped": skipped, "failed": failed}),
    ))
}

/// Declarative export of the fleet config (no secrets): groups, routing
/// profiles, nodes and inbounds, referenced by stable names.
async fn config_export(State(st): State<AppState>) -> Result<Json<JsonValue>, ApiError> {
    let node_map: std::collections::HashMap<Uuid, String> = repo::list_nodes(&st.pool)
        .await?
        .into_iter()
        .map(|n| (n.id, n.name))
        .collect();
    let groups: Vec<JsonValue> = repo::list_node_groups(&st.pool)
        .await?
        .into_iter()
        .map(|g| json!({"name": g.name, "note": g.note, "is_default": g.is_default}))
        .collect();
    let profiles: Vec<JsonValue> = repo::list_routing_profiles(&st.pool)
        .await?
        .into_iter()
        .map(|p| {
            json!({
                "name": p.name, "block_ads": p.block_ads, "block_adult": p.block_adult,
                "block_gambling": p.block_gambling, "direct_private": p.direct_private,
                "direct_geosite": p.direct_geosite, "direct_geoip": p.direct_geoip,
                "final_proxy": p.final_proxy, "is_default": p.is_default,
                "blocked_domains": p.blocked_domains, "direct_domains": p.direct_domains,
                "proxy_domains": p.proxy_domains, "notes": p.notes
            })
        })
        .collect();
    let nodes: Vec<JsonValue> = repo::list_nodes(&st.pool)
        .await?
        .into_iter()
        .map(|n| {
            json!({
                "name": n.name, "address": n.address, "tls_server_name": n.tls_server_name,
                "grpc_port": n.grpc_port, "transport": n.transport,
                "monthly_cost_cents": n.monthly_cost_cents, "enabled": n.enabled
            })
        })
        .collect();
    let mut inbounds: Vec<JsonValue> = Vec::new();
    for (node_id, node_name) in &node_map {
        for i in repo::node_inbounds(&st.pool, *node_id)
            .await
            .unwrap_or_default()
        {
            inbounds.push(json!({
                "node": node_name, "tag": i.tag, "kind": i.kind, "core": i.core,
                "listen_port": i.listen_port, "network": i.network, "tls_enabled": i.tls_enabled,
                "reality": i.reality, "server_name": i.server_name, "enabled": i.enabled
            }));
        }
    }
    Ok(Json(json!({
        "version": 1, "groups": groups, "routing_profiles": profiles,
        "nodes": nodes, "inbounds": inbounds
    })))
}

#[derive(Deserialize)]
struct ConfigDoc {
    #[serde(default)]
    groups: Vec<crate::db::models::NewNodeGroup>,
    #[serde(default)]
    routing_profiles: Vec<crate::db::models::NewRoutingProfile>,
    #[serde(default)]
    nodes: Vec<crate::db::models::NewNode>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default)]
    prune: bool,
}

/// Apply a declarative config: create missing, update existing (matched by
/// name), optionally prune extras. Inbounds are export-only for now. `dry_run`
/// returns the plan without touching anything.
async fn config_apply(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(doc): Json<ConfigDoc>,
) -> Result<Json<JsonValue>, ApiError> {
    let dry = doc.dry_run;
    let (mut gc, mut gu, mut gd) = (0u32, 0u32, 0u32);
    let (mut pc, mut pu, mut pd) = (0u32, 0u32, 0u32);
    let (mut nc, mut nu, mut nd) = (0u32, 0u32, 0u32);

    // groups
    let existing_groups = repo::list_node_groups(&st.pool).await?;
    let desired_group_names: std::collections::HashSet<String> = doc
        .groups
        .iter()
        .map(|g| g.name.trim().to_lowercase())
        .collect();
    for spec in &doc.groups {
        match existing_groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(spec.name.trim()))
        {
            Some(cur) => {
                if cur.note != spec.note {
                    if !dry {
                        repo::update_node_group(
                            &st.pool,
                            cur.id,
                            &crate::db::models::UpdateNodeGroup {
                                name: None,
                                note: Some(spec.note.clone()),
                            },
                        )
                        .await?;
                    }
                    gu += 1;
                }
            }
            None => {
                if !dry {
                    repo::create_node_group(&st.pool, spec).await?;
                }
                gc += 1;
            }
        }
    }
    if doc.prune {
        for cur in &existing_groups {
            if !cur.is_default && !desired_group_names.contains(&cur.name.to_lowercase()) {
                if !dry {
                    repo::delete_node_group(&st.pool, cur.id).await?;
                }
                gd += 1;
            }
        }
    }

    // routing profiles (idempotent update to desired values)
    let existing_profiles = repo::list_routing_profiles(&st.pool).await?;
    let desired_profile_names: std::collections::HashSet<String> = doc
        .routing_profiles
        .iter()
        .map(|p| p.name.trim().to_lowercase())
        .collect();
    for spec in &doc.routing_profiles {
        let update = crate::db::models::UpdateRoutingProfile {
            name: Some(spec.name.clone()),
            block_ads: Some(spec.block_ads),
            direct_private: Some(spec.direct_private),
            direct_geosite: Some(spec.direct_geosite.clone()),
            direct_geoip: Some(spec.direct_geoip.clone()),
            final_proxy: Some(spec.final_proxy),
            is_default: Some(spec.is_default),
            notes: Some(spec.notes.clone()),
            block_adult: Some(spec.block_adult),
            block_gambling: Some(spec.block_gambling),
            blocked_domains: Some(spec.blocked_domains.clone()),
            direct_domains: Some(spec.direct_domains.clone()),
            proxy_domains: Some(spec.proxy_domains.clone()),
            app_rules: None,
            dns_doh: None,
            dns_fakeip: None,
            dns_block_plain: None,
        };
        match existing_profiles
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(spec.name.trim()))
        {
            Some(cur) => {
                if !dry {
                    repo::update_routing_profile(&st.pool, cur.id, &update).await?;
                }
                pu += 1;
            }
            None => {
                if !dry {
                    repo::create_routing_profile(&st.pool, spec).await?;
                }
                pc += 1;
            }
        }
    }
    if doc.prune {
        for cur in &existing_profiles {
            if !cur.is_default && !desired_profile_names.contains(&cur.name.to_lowercase()) {
                if !dry {
                    repo::delete_routing_profile(&st.pool, cur.id).await?;
                }
                pd += 1;
            }
        }
    }

    // nodes
    let existing_nodes = repo::list_nodes(&st.pool).await?;
    let desired_node_names: std::collections::HashSet<String> = doc
        .nodes
        .iter()
        .map(|n| n.name.trim().to_lowercase())
        .collect();
    for spec in doc.nodes {
        match existing_nodes
            .iter()
            .find(|n| n.name.eq_ignore_ascii_case(spec.name.trim()))
        {
            Some(cur) => {
                if !dry {
                    repo::update_node(
                        &st.pool,
                        cur.id,
                        UpdateNode {
                            name: None,
                            address: Some(spec.address),
                            tls_server_name: Some(spec.tls_server_name),
                            grpc_port: Some(spec.grpc_port),
                            transport: Some(spec.transport),
                            enabled: None,
                            extra_addresses: None,
                            maintenance: None,
                            monthly_cost_cents: Some(spec.monthly_cost_cents),
                        },
                    )
                    .await?;
                }
                nu += 1;
            }
            None => {
                if !dry {
                    repo::create_node(&st.pool, spec).await?;
                }
                nc += 1;
            }
        }
    }
    if doc.prune {
        for cur in &existing_nodes {
            if !desired_node_names.contains(&cur.name.to_lowercase()) {
                if !dry {
                    repo::delete_node(&st.pool, cur.id).await?;
                }
                nd += 1;
            }
        }
    }

    if !dry {
        audit(&st, &identity, "apply", "config", None, json!({})).await;
    }
    Ok(Json(json!({
        "dry_run": dry, "prune": doc.prune,
        "groups": {"created": gc, "updated": gu, "deleted": gd},
        "routing_profiles": {"created": pc, "updated": pu, "deleted": pd},
        "nodes": {"created": nc, "updated": nu, "deleted": nd},
        "note": "inbounds are export-only; apply them via /inbounds"
    })))
}

async fn list_audit(State(st): State<AppState>) -> Result<Json<Vec<AuditEvent>>, ApiError> {
    let limit = repo::setting_i64(&st.pool, "audit_retention", DEFAULT_AUDIT_RETENTION)
        .await
        .clamp(10, 5000) as i64;
    Ok(Json(repo::list_audit_events(&st.pool, limit).await?))
}

/// Recompute the audit hash chain and report whether it is intact. Legacy
/// pre-chain rows (no stored hash) are carried but not verified.
async fn verify_audit_chain(State(st): State<AppState>) -> Result<Json<JsonValue>, ApiError> {
    let rows = repo::audit_chain_rows(&st.pool).await?;
    let mut prev: Option<Vec<u8>> = None;
    let mut checked = 0u64;
    let mut broken_at: Option<i64> = None;
    for (id, actor, action, rtype, rid, details, created, stored) in rows {
        match stored {
            None => prev = None,
            Some(stored_hash) => {
                let expected = auth::audit_chain_hash(
                    prev.as_deref(),
                    id,
                    actor.as_deref(),
                    &action,
                    &rtype,
                    rid.as_deref(),
                    &details.to_string(),
                    created.timestamp_micros(),
                );
                checked += 1;
                if expected != stored_hash {
                    broken_at = Some(id);
                    break;
                }
                prev = Some(stored_hash);
            }
        }
    }
    Ok(Json(json!({
        "intact": broken_at.is_none(),
        "verified_entries": checked,
        "broken_at": broken_at
    })))
}

/// GDPR data export: everything honey holds about a user (no admin action logs,
/// which are retained separately as a lawful record).
async fn gdpr_export(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let user = owned_user(&st, &identity, id).await?;
    let group_ids = repo::user_group_ids(&st.pool, id).await.unwrap_or_default();
    let all_groups = repo::list_node_groups(&st.pool).await.unwrap_or_default();
    let groups: Vec<String> = all_groups
        .into_iter()
        .filter(|g| group_ids.contains(&g.id))
        .map(|g| g.name)
        .collect();
    let dump = json!({
        "exported_at": Utc::now(),
        "user": {
            "id": user.id,
            "username": user.username,
            "uuid": user.uuid,
            "enabled": user.enabled,
            "traffic_limit_bytes": user.traffic_limit_bytes,
            "used_traffic_bytes": user.used_traffic_bytes,
            "expires_at": user.expires_at,
            "created_at": user.created_at,
            "subscription_alias": user.subscription_alias,
            "quota_interval": user.quota_interval,
            "routing_profile_id": user.routing_profile_id,
        },
        "group_access": groups,
        "note": "Subscription tokens are stored hashed and are not exportable; the sub link is revocable via rotation."
    });
    audit(&st, &identity, "gdpr_export", "user", Some(id), json!({})).await;
    let body = serde_json::to_string_pretty(&dump).unwrap_or_default();
    let mut response = (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body,
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"gdpr-{id}.json\"")) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(response)
}

/// GDPR right-to-erasure: delete the user and all their honey data (cascade),
/// distinct from an ordinary admin delete for compliance workflows.
async fn gdpr_erase(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    owned_user(&st, &identity, id).await?;
    let nodes = repo::user_node_ids(&st.pool, id).await?;
    if !repo::delete_user(&st.pool, id).await? {
        return Err(ApiError::not_found("user not found"));
    }
    push_nodes(&st, nodes).await;
    audit(&st, &identity, "gdpr_erase", "user", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct LogQuery {
    limit: Option<usize>,
    level: Option<String>,
    code: Option<String>,
    #[serde(rename = "q")]
    query: Option<String>,
}

fn validate_log_query(query: &LogQuery) -> Result<(), ApiError> {
    if query
        .level
        .as_deref()
        .is_some_and(|value| !matches!(value, "error" | "warn" | "info" | "debug" | "trace"))
    {
        return Err(ApiError::bad_request("unsupported log level"));
    }
    if query.code.as_deref().is_some_and(|value| {
        let mut chars = value.trim().chars();
        chars.next() != Some('M')
            || chars.clone().count() != 4
            || !chars.all(|c| c.is_ascii_digit())
    }) {
        return Err(ApiError::bad_request("invalid log code filter"));
    }
    if query
        .query
        .as_deref()
        .is_some_and(|value| value.trim().len() > 128)
    {
        return Err(ApiError::bad_request("log search is too long"));
    }
    Ok(())
}

/// live tail of the master's own runtime log (the `M####` codes), newest first.
async fn system_logs(
    State(st): State<AppState>,
    Query(q): Query<LogQuery>,
) -> Result<Json<Vec<crate::logbuf::LogRecord>>, ApiError> {
    validate_log_query(&q)?;
    let default = repo::setting_i64(&st.pool, "runtime_log_limit", DEFAULT_RUNTIME_LOG_LIMIT).await;
    let limit = q.limit.unwrap_or(default.clamp(10, 5000) as usize);
    let level = q.level.as_deref();
    let code = q.code.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let query = q.query.as_deref().map(str::trim).filter(|v| !v.is_empty());
    Ok(Json(crate::logbuf::search(limit, level, code, query)))
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TrafficAnalyticsQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    bucket: Option<String>,
    node_id: Option<Uuid>,
    user_id: Option<Uuid>,
    core: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct TrafficRange {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: &'static str,
}

fn validate_traffic_query(
    query: &TrafficAnalyticsQuery,
    now: DateTime<Utc>,
) -> Result<TrafficRange, ApiError> {
    if query
        .core
        .as_deref()
        .is_some_and(|core| !matches!(core, "singbox" | "xray"))
    {
        return Err(ApiError::bad_request("core must be singbox or xray"));
    }
    let to = query.to.unwrap_or(now);
    let from = query.from.unwrap_or(to - Duration::hours(24));
    let span = to - from;
    if span <= Duration::zero() {
        return Err(ApiError::bad_request("traffic range must be positive"));
    }
    if span > Duration::days(366) {
        return Err(ApiError::bad_request(
            "traffic range must not exceed 366 days",
        ));
    }
    let bucket = match query.bucket.as_deref() {
        None if span <= Duration::days(7) => "hour",
        None => "day",
        Some("hour") => "hour",
        Some("day") => "day",
        Some(_) => return Err(ApiError::bad_request("bucket must be hour or day")),
    };
    if bucket == "hour" && span > Duration::days(31) {
        return Err(ApiError::bad_request(
            "hour buckets are limited to a 31-day range",
        ));
    }
    Ok(TrafficRange { from, to, bucket })
}

#[derive(Debug, Serialize)]
struct TrafficSummaryView {
    up_bytes: i64,
    down_bytes: i64,
    total_bytes: i64,
    previous_total_bytes: i64,
    change_percent: Option<f64>,
}

#[derive(Debug, Serialize)]
struct TrafficAnalyticsView {
    scope: &'static str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    bucket: &'static str,
    retention_days: i64,
    summary: TrafficSummaryView,
    series: Vec<TrafficSeriesPoint>,
    top_users: Vec<TrafficRank>,
    top_nodes: Vec<TrafficRank>,
    cores: Vec<TrafficCoreBreakdown>,
    health: Option<FleetHealthSummary>,
}

fn traffic_change_percent(current: i64, previous: i64) -> Option<f64> {
    if previous <= 0 {
        return None;
    }
    Some(((current - previous) as f64 / previous as f64) * 100.0)
}

async fn build_traffic_analytics(
    st: &AppState,
    identity: &Identity,
    query: &TrafficAnalyticsQuery,
) -> Result<TrafficAnalyticsView, ApiError> {
    let range = validate_traffic_query(query, Utc::now())?;
    let reseller = identity.is_reseller();
    if reseller && query.node_id.is_some() {
        return Err(ApiError::forbidden(
            "node filters are outside reseller scope",
        ));
    }
    if let Some(user_id) = query.user_id {
        let _ = owned_user(st, identity, user_id).await?;
    }
    let creator = if reseller {
        Some(
            identity
                .admin_id
                .ok_or_else(|| ApiError::forbidden("reseller identity has no account"))?,
        )
    } else {
        None
    };
    let core = query.core.as_deref();
    let previous_from = range.from - (range.to - range.from);
    let (current, previous, series, top_users, cores) = tokio::try_join!(
        repo::traffic_totals(
            &st.pool,
            range.from,
            range.to,
            query.node_id,
            query.user_id,
            core,
            creator,
        ),
        repo::traffic_totals(
            &st.pool,
            previous_from,
            range.from,
            query.node_id,
            query.user_id,
            core,
            creator,
        ),
        repo::traffic_series(
            &st.pool,
            range.from,
            range.to,
            range.bucket,
            query.node_id,
            query.user_id,
            core,
            creator,
        ),
        repo::traffic_top_users(
            &st.pool,
            range.from,
            range.to,
            query.node_id,
            query.user_id,
            core,
            creator,
        ),
        repo::traffic_by_core(
            &st.pool,
            range.from,
            range.to,
            query.node_id,
            query.user_id,
            core,
            creator,
        ),
    )?;
    let (top_nodes, health) = if reseller {
        (Vec::new(), None)
    } else {
        let (nodes, health) = tokio::try_join!(
            repo::traffic_top_nodes(
                &st.pool,
                range.from,
                range.to,
                query.node_id,
                query.user_id,
                core,
            ),
            repo::fleet_health_summary(&st.pool),
        )?;
        (nodes, Some(health))
    };
    let total = current.0.saturating_add(current.1);
    let previous_total = previous.0.saturating_add(previous.1);
    let retention_days = repo::setting_i64(
        &st.pool,
        "traffic_history_days",
        crate::stats::DEFAULT_TRAFFIC_HISTORY_DAYS,
    )
    .await
    .clamp(7, 3650);
    Ok(TrafficAnalyticsView {
        scope: if reseller { "reseller" } else { "fleet" },
        from: range.from,
        to: range.to,
        bucket: range.bucket,
        retention_days,
        summary: TrafficSummaryView {
            up_bytes: current.0,
            down_bytes: current.1,
            total_bytes: total,
            previous_total_bytes: previous_total,
            change_percent: traffic_change_percent(total, previous_total),
        },
        series,
        top_users,
        top_nodes,
        cores,
        health,
    })
}

async fn traffic_analytics(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<TrafficAnalyticsQuery>,
) -> Result<Json<TrafficAnalyticsView>, ApiError> {
    Ok(Json(build_traffic_analytics(&st, &identity, &query).await?))
}

fn report_bytes(value: i64) -> String {
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

fn report_rows(rows: &[crate::db::models::TrafficRank]) -> String {
    if rows.is_empty() {
        return "<tr><td colspan=\"4\" class=\"muted\">no traffic in this period</td></tr>".into();
    }
    rows.iter()
        .map(|r| {
            format!(
                "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                html_escape(&r.name),
                report_bytes(r.up_bytes + r.down_bytes),
                report_bytes(r.up_bytes),
                report_bytes(r.down_bytes)
            )
        })
        .collect()
}

/// A printable period report (fleet + traffic aggregates). Rendered as a
/// self-contained HTML document so the browser's "Print → Save as PDF" produces
/// the PDF — no server-side PDF toolchain. Reseller-scoped like the analytics API.
async fn period_report(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<TrafficAnalyticsQuery>,
) -> Result<Response, ApiError> {
    let data = build_traffic_analytics(&st, &identity, &query).await?;
    let brand = repo::get_branding(&st.pool)
        .await
        .map(|b| b.brand_name)
        .unwrap_or_else(|_| "honey".to_string());

    let change = match data.summary.change_percent {
        Some(p) => format!("{p:+.1}% vs previous period"),
        None => "no comparable previous period".to_string(),
    };
    let health = match &data.health {
        Some(h) => format!(
            "<div class=\"cards\"><div class=\"card\"><small>Nodes online</small><b>{}/{}</b></div>\
             <div class=\"card\"><small>Failed pushes</small><b>{}</b></div>\
             <div class=\"card\"><small>Unreachable endpoints</small><b>{}</b></div></div>",
            h.nodes_online, h.nodes_total, h.failed_pushes, h.unreachable_endpoints
        ),
        None => String::new(),
    };
    let cores = if data.cores.is_empty() {
        "<tr><td colspan=\"4\" class=\"muted\">no core traffic in this period</td></tr>".to_string()
    } else {
        data.cores
            .iter()
            .map(|c| {
                format!(
                    "<tr><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                    html_escape(&c.core),
                    report_bytes(c.up_bytes + c.down_bytes),
                    report_bytes(c.up_bytes),
                    report_bytes(c.down_bytes)
                )
            })
            .collect()
    };

    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{brand} — period report</title><style>\
*{{box-sizing:border-box}}body{{margin:0;padding:32px;font:14px/1.5 system-ui,-apple-system,sans-serif;color:#111;background:#fff}}\
h1{{font-size:22px;margin:0 0 4px}}h2{{font-size:15px;margin:28px 0 8px}}\
.sub{{color:#666;font-size:12px;margin-bottom:20px}}\
.cards{{display:flex;gap:10px;flex-wrap:wrap;margin:10px 0 4px}}\
.card{{flex:1 1 150px;border:1px solid #ddd;border-radius:8px;padding:12px}}\
.card small{{display:block;color:#666;font-size:11px;text-transform:uppercase;letter-spacing:.05em}}\
.card b{{display:block;font-size:20px;margin-top:4px}}\
table{{width:100%;border-collapse:collapse;font-size:13px}}\
th,td{{text-align:left;padding:7px 8px;border-bottom:1px solid #e5e5e5}}\
th{{color:#666;font-weight:600;font-size:11px;text-transform:uppercase;letter-spacing:.04em}}\
td.n,th.n{{text-align:right;font-variant-numeric:tabular-nums}}\
.muted{{color:#888}}footer{{margin-top:28px;color:#888;font-size:11px}}\
@media print{{body{{padding:0}}.card{{break-inside:avoid}}table{{break-inside:auto}}tr{{break-inside:avoid}}}}\
</style></head><body>\
<h1>{brand} — period report</h1>\
<div class=\"sub\">{from} → {to} · bucket {bucket} · scope {scope} · generated {now}</div>\
{health}\
<h2>Traffic</h2>\
<div class=\"cards\">\
<div class=\"card\"><small>Total</small><b>{total}</b></div>\
<div class=\"card\"><small>Upload</small><b>{up}</b></div>\
<div class=\"card\"><small>Download</small><b>{down}</b></div>\
<div class=\"card\"><small>Change</small><b style=\"font-size:14px\">{change}</b></div>\
</div>\
<h2>Top users</h2><table><thead><tr><th>User</th><th class=\"n\">Total</th><th class=\"n\">Up</th><th class=\"n\">Down</th></tr></thead><tbody>{users}</tbody></table>\
<h2>Top nodes</h2><table><thead><tr><th>Node</th><th class=\"n\">Total</th><th class=\"n\">Up</th><th class=\"n\">Down</th></tr></thead><tbody>{nodes}</tbody></table>\
<h2>Core split</h2><table><thead><tr><th>Core</th><th class=\"n\">Total</th><th class=\"n\">Up</th><th class=\"n\">Down</th></tr></thead><tbody>{cores}</tbody></table>\
<footer>Counters are agent-reported per user and core; protocol/transport attribution is not claimed. Print this page (Ctrl/Cmd+P) to save it as PDF.</footer>\
</body></html>",
        brand = html_escape(&brand),
        from = data.from.format("%Y-%m-%d %H:%M UTC"),
        to = data.to.format("%Y-%m-%d %H:%M UTC"),
        bucket = data.bucket,
        scope = data.scope,
        now = Utc::now().format("%Y-%m-%d %H:%M UTC"),
        total = report_bytes(data.summary.total_bytes),
        up = report_bytes(data.summary.up_bytes),
        down = report_bytes(data.summary.down_bytes),
        users = report_rows(&data.top_users),
        nodes = report_rows(&data.top_nodes),
    );

    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // static document: no scripts at all, inline styles only.
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
    );
    Ok(response)
}

async fn traffic_analytics_csv(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<TrafficAnalyticsQuery>,
) -> Result<Response, ApiError> {
    let view = build_traffic_analytics(&st, &identity, &query).await?;
    let mut body = String::from("bucket,upload_bytes,download_bytes,total_bytes\n");
    for point in view.series {
        body.push_str(&format!(
            "{},{},{},{}\n",
            point.bucket.to_rfc3339(),
            point.up_bytes,
            point.down_bytes,
            point.up_bytes.saturating_add(point.down_bytes)
        ));
    }
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"honey-traffic.csv\""),
    );
    Ok(response)
}

// --- runtime-editable operator settings -----------------------------------

const DEFAULT_RECONCILE_SECS: i64 = 30;
const DEFAULT_AUDIT_RETENTION: i64 = 200;
const DEFAULT_RUNTIME_LOG_LIMIT: i64 = 200;
const DEFAULT_INBOUND_CORE: &str = "singbox";
const DEFAULT_SUBSCRIPTION_GUARD_MAX_REQUESTS: i64 = 120;
const DEFAULT_SUBSCRIPTION_GUARD_WINDOW_SECS: i64 = 60;
const DEFAULT_SUBSCRIPTION_GUARD_BLOCK_SECS: i64 = 300;

fn subscription_guard_config_from_map(
    map: &std::collections::HashMap<String, String>,
) -> crate::ratelimit::SubscriptionLimitConfig {
    let int = |key: &str, default: i64| {
        map.get(key)
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(default)
    };
    let enabled = map
        .get("subscription_guard_enabled")
        .and_then(|value| value.trim().parse::<bool>().ok())
        .unwrap_or(true);
    crate::ratelimit::SubscriptionLimitConfig {
        enabled,
        max_requests: int(
            "subscription_guard_max_requests",
            DEFAULT_SUBSCRIPTION_GUARD_MAX_REQUESTS,
        )
        .clamp(10, 10_000) as u32,
        window: std::time::Duration::from_secs(
            int(
                "subscription_guard_window_secs",
                DEFAULT_SUBSCRIPTION_GUARD_WINDOW_SECS,
            )
            .clamp(10, 3600) as u64,
        ),
        block: std::time::Duration::from_secs(
            int(
                "subscription_guard_block_secs",
                DEFAULT_SUBSCRIPTION_GUARD_BLOCK_SECS,
            )
            .clamp(10, 86_400) as u64,
        ),
    }
}

async fn subscription_guard_config(pool: &PgPool) -> crate::ratelimit::SubscriptionLimitConfig {
    match repo::all_settings(pool).await {
        Ok(values) => subscription_guard_config_from_map(&values.into_iter().collect()),
        Err(error) => {
            tracing::warn!(code = "M1702", %error, "subscription guard settings unavailable; defaults applied");
            crate::ratelimit::SubscriptionLimitConfig::default()
        }
    }
}

#[derive(Serialize)]
struct SettingsView {
    reconcile_secs: i64,
    auto_push_enabled: bool,
    audit_retention: i64,
    runtime_log_limit: i64,
    traffic_history_days: i64,
    default_inbound_core: String,
    default_subscription_title: String,
    default_subscription_description: String,
    subscription_support_url: String,
    subscription_guard_enabled: bool,
    subscription_guard_max_requests: u32,
    subscription_guard_window_secs: u64,
    subscription_guard_block_secs: u64,
    subscription_guard_allowed_total: u64,
    subscription_guard_blocked_total: u64,
    subscription_guard_active_buckets: usize,
    subscription_guard_last_blocked_at: Option<i64>,
    subscription_guard_recent_blocks: i64,
    anomaly_enabled: bool,
    anomaly_factor_pct: i64,
    anomaly_min_mib: i64,
    anomaly_baseline_hours: i64,
    anomaly_min_history_hours: i64,
    device_limit_enforce: bool,
    preflight_gate: String,
    cdn_rotate_enabled: bool,
    cdn_rotate_margin_pct: i64,
    self_update_enabled: bool,
    secret_backend: String,
    secret_encryption_enabled: bool,
}

async fn get_settings(State(st): State<AppState>) -> Result<Json<SettingsView>, ApiError> {
    let map: std::collections::HashMap<String, String> =
        repo::all_settings(&st.pool).await?.into_iter().collect();
    let int = |key: &str, default: i64| {
        map.get(key)
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(default)
    };
    let guard = subscription_guard_config_from_map(&map);
    let guard_stats = st.subscription_limiter.stats().await;
    let (recent_blocks, _) = repo::subscription_abuse_summary(&st.pool).await?;
    Ok(Json(SettingsView {
        reconcile_secs: int("reconcile_secs", DEFAULT_RECONCILE_SECS),
        auto_push_enabled: int("auto_push_enabled", 1) != 0,
        audit_retention: int("audit_retention", DEFAULT_AUDIT_RETENTION),
        runtime_log_limit: int("runtime_log_limit", DEFAULT_RUNTIME_LOG_LIMIT),
        traffic_history_days: int(
            "traffic_history_days",
            crate::stats::DEFAULT_TRAFFIC_HISTORY_DAYS,
        )
        .clamp(7, 3650),
        default_inbound_core: map
            .get("default_inbound_core")
            .cloned()
            .unwrap_or_else(|| DEFAULT_INBOUND_CORE.to_string()),
        default_subscription_title: map
            .get("default_subscription_title")
            .cloned()
            .unwrap_or_default(),
        default_subscription_description: map
            .get("default_subscription_description")
            .cloned()
            .unwrap_or_default(),
        subscription_support_url: map
            .get("subscription_support_url")
            .cloned()
            .unwrap_or_default(),
        subscription_guard_enabled: guard.enabled,
        subscription_guard_max_requests: guard.max_requests,
        subscription_guard_window_secs: guard.window.as_secs(),
        subscription_guard_block_secs: guard.block.as_secs(),
        subscription_guard_allowed_total: guard_stats.allowed_total,
        subscription_guard_blocked_total: guard_stats.blocked_total,
        subscription_guard_active_buckets: guard_stats.active_buckets,
        subscription_guard_last_blocked_at: guard_stats.last_blocked_at,
        subscription_guard_recent_blocks: recent_blocks,
        anomaly_enabled: int("anomaly_enabled", 1) != 0,
        anomaly_factor_pct: int("anomaly_factor_pct", 500).clamp(150, 100_000),
        anomaly_min_mib: int("anomaly_min_mib", 5120).max(0),
        anomaly_baseline_hours: int("anomaly_baseline_hours", 72).clamp(6, 720),
        anomaly_min_history_hours: int("anomaly_min_history_hours", 6).clamp(1, 240),
        device_limit_enforce: int("device_limit_enforce", 0) != 0,
        preflight_gate: map
            .get("preflight_gate")
            .map(|v| v.trim().to_string())
            .filter(|v| matches!(v.as_str(), "off" | "warn" | "block"))
            .unwrap_or_else(|| "warn".to_string()),
        cdn_rotate_enabled: int("cdn_rotate_enabled", 0) != 0,
        cdn_rotate_margin_pct: int("cdn_rotate_margin_pct", 30).clamp(1, 90),
        self_update_enabled: int("self_update_enabled", 0) != 0,
        secret_backend: crate::secret_source::active_backend().to_string(),
        secret_encryption_enabled: crate::secret::is_enabled(),
    }))
}

#[derive(Deserialize)]
struct UpdateSettings {
    #[serde(default)]
    reconcile_secs: Option<i64>,
    #[serde(default)]
    auto_push_enabled: Option<bool>,
    #[serde(default)]
    audit_retention: Option<i64>,
    #[serde(default)]
    runtime_log_limit: Option<i64>,
    #[serde(default)]
    traffic_history_days: Option<i64>,
    #[serde(default)]
    default_inbound_core: Option<String>,
    #[serde(default)]
    default_subscription_title: Option<String>,
    #[serde(default)]
    default_subscription_description: Option<String>,
    #[serde(default)]
    subscription_support_url: Option<String>,
    #[serde(default)]
    subscription_guard_enabled: Option<bool>,
    #[serde(default)]
    subscription_guard_max_requests: Option<i64>,
    #[serde(default)]
    subscription_guard_window_secs: Option<i64>,
    #[serde(default)]
    subscription_guard_block_secs: Option<i64>,
    #[serde(default)]
    anomaly_enabled: Option<bool>,
    #[serde(default)]
    anomaly_factor_pct: Option<i64>,
    #[serde(default)]
    anomaly_min_mib: Option<i64>,
    #[serde(default)]
    anomaly_baseline_hours: Option<i64>,
    #[serde(default)]
    anomaly_min_history_hours: Option<i64>,
    #[serde(default)]
    device_limit_enforce: Option<bool>,
    #[serde(default)]
    preflight_gate: Option<String>,
    #[serde(default)]
    cdn_rotate_enabled: Option<bool>,
    #[serde(default)]
    cdn_rotate_margin_pct: Option<i64>,
    #[serde(default)]
    self_update_enabled: Option<bool>,
}

async fn update_settings(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<UpdateSettings>,
) -> Result<Json<SettingsView>, ApiError> {
    if let Some(v) = input.reconcile_secs {
        repo::set_setting(&st.pool, "reconcile_secs", &v.clamp(5, 86_400).to_string()).await?;
    }
    if let Some(v) = input.auto_push_enabled {
        repo::set_setting(&st.pool, "auto_push_enabled", if v { "1" } else { "0" }).await?;
    }
    if let Some(v) = input.audit_retention {
        repo::set_setting(&st.pool, "audit_retention", &v.clamp(10, 5000).to_string()).await?;
    }
    if let Some(v) = input.runtime_log_limit {
        repo::set_setting(
            &st.pool,
            "runtime_log_limit",
            &v.clamp(10, 5000).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.traffic_history_days {
        repo::set_setting(
            &st.pool,
            "traffic_history_days",
            &v.clamp(7, 3650).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.default_inbound_core {
        if v != "singbox" && v != "xray" {
            return Err(ApiError::bad_request("core must be singbox or xray"));
        }
        repo::set_setting(&st.pool, "default_inbound_core", &v).await?;
    }
    if let Some(v) = input.default_subscription_title {
        if !v.trim().is_empty() {
            validate_subscription_title(Some(v.trim()))?;
        }
        repo::set_setting(&st.pool, "default_subscription_title", v.trim()).await?;
    }
    if let Some(v) = input.default_subscription_description {
        let value = v.trim();
        if value.chars().count() > 200 {
            return Err(ApiError::bad_request(
                "default_subscription_description must be at most 200 characters",
            ));
        }
        repo::set_setting(&st.pool, "default_subscription_description", value).await?;
    }
    if let Some(v) = input.subscription_support_url {
        let value = v.trim();
        if !value.is_empty() && !(value.starts_with("https://") || value.starts_with("tg://")) {
            return Err(ApiError::bad_request(
                "subscription_support_url must use https:// or tg://",
            ));
        }
        repo::set_setting(&st.pool, "subscription_support_url", value).await?;
    }
    if let Some(v) = input.subscription_guard_enabled {
        repo::set_setting(&st.pool, "subscription_guard_enabled", &v.to_string()).await?;
    }
    if let Some(v) = input.subscription_guard_max_requests {
        repo::set_setting(
            &st.pool,
            "subscription_guard_max_requests",
            &v.clamp(10, 10_000).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.subscription_guard_window_secs {
        repo::set_setting(
            &st.pool,
            "subscription_guard_window_secs",
            &v.clamp(10, 3600).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.subscription_guard_block_secs {
        repo::set_setting(
            &st.pool,
            "subscription_guard_block_secs",
            &v.clamp(10, 86_400).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.anomaly_enabled {
        repo::set_setting(&st.pool, "anomaly_enabled", if v { "1" } else { "0" }).await?;
    }
    if let Some(v) = input.anomaly_factor_pct {
        repo::set_setting(
            &st.pool,
            "anomaly_factor_pct",
            &v.clamp(150, 100_000).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.anomaly_min_mib {
        repo::set_setting(
            &st.pool,
            "anomaly_min_mib",
            &v.clamp(0, 10_485_760).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.anomaly_baseline_hours {
        repo::set_setting(
            &st.pool,
            "anomaly_baseline_hours",
            &v.clamp(6, 720).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.anomaly_min_history_hours {
        repo::set_setting(
            &st.pool,
            "anomaly_min_history_hours",
            &v.clamp(1, 240).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.device_limit_enforce {
        repo::set_setting(&st.pool, "device_limit_enforce", if v { "1" } else { "0" }).await?;
    }
    if let Some(v) = input.preflight_gate {
        if !matches!(v.trim(), "off" | "warn" | "block") {
            return Err(ApiError::bad_request(
                "preflight_gate must be off, warn or block",
            ));
        }
        repo::set_setting(&st.pool, "preflight_gate", v.trim()).await?;
    }
    if let Some(v) = input.cdn_rotate_enabled {
        repo::set_setting(&st.pool, "cdn_rotate_enabled", if v { "1" } else { "0" }).await?;
    }
    if let Some(v) = input.cdn_rotate_margin_pct {
        repo::set_setting(
            &st.pool,
            "cdn_rotate_margin_pct",
            &v.clamp(1, 90).to_string(),
        )
        .await?;
    }
    if let Some(v) = input.self_update_enabled {
        repo::set_setting(&st.pool, "self_update_enabled", if v { "1" } else { "0" }).await?;
    }
    audit(&st, &identity, "update", "settings", None, json!({})).await;
    get_settings(State(st)).await
}

// --- persistent in-app notifications --------------------------------------

#[derive(Deserialize, Default)]
struct NotificationQuery {
    severity: Option<String>,
    event: Option<String>,
    #[serde(default)]
    unread: bool,
    limit: Option<i64>,
}

fn validate_notification_query(query: &NotificationQuery) -> Result<(), ApiError> {
    if query
        .severity
        .as_deref()
        .is_some_and(|value| !matches!(value, "critical" | "warning" | "info"))
    {
        return Err(ApiError::bad_request("unsupported notification severity"));
    }
    if query.event.as_deref().is_some_and(|value| {
        !matches!(
            value,
            "node_down" | "push_failed" | "cert_expiry" | "quota_reset" | "subscription_abuse"
        )
    }) {
        return Err(ApiError::bad_request("unsupported notification event"));
    }
    Ok(())
}

async fn list_notifications(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Query(query): Query<NotificationQuery>,
) -> Result<Json<Vec<SystemNotificationView>>, ApiError> {
    validate_notification_query(&query)?;
    Ok(Json(
        repo::list_system_notifications(
            &st.pool,
            identity.admin_id,
            query.severity.as_deref(),
            query.event.as_deref(),
            query.unread,
            query.limit.unwrap_or(50),
        )
        .await?,
    ))
}

async fn notification_unread_count(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(json!({
        "unread": repo::count_unread_system_notifications(&st.pool, identity.admin_id).await?
    })))
}

async fn mark_notification_read(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let admin_id = session_account(&identity)?;
    if !repo::mark_system_notification_read(&st.pool, admin_id, id).await? {
        return Err(ApiError::not_found("notification not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_all_notifications_read(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<JsonValue>, ApiError> {
    let admin_id = session_account(&identity)?;
    let marked = repo::mark_all_system_notifications_read(&st.pool, admin_id).await?;
    Ok(Json(json!({"marked": marked})))
}

// --- managed domains --------------------------------------------------------

async fn list_domains(State(st): State<AppState>) -> Result<Json<Vec<ManagedDomain>>, ApiError> {
    Ok(Json(repo::list_managed_domains(&st.pool).await?))
}

async fn get_domain(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ManagedDomain>, ApiError> {
    repo::get_managed_domain(&st.pool, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("domain not found"))
}

async fn create_domain(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewManagedDomain>,
) -> Result<Json<ManagedDomain>, ApiError> {
    let host = input.host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(|c: char| c == '/' || c == ':' || c.is_whitespace()) {
        return Err(ApiError::bad_request(
            "host must be a bare domain (no scheme, port or path)",
        ));
    }
    if let Some(node_id) = input.node_id {
        if repo::get_node(&st.pool, node_id).await?.is_none() {
            return Err(ApiError::bad_request("node_id does not exist"));
        }
    }
    let domain = repo::create_managed_domain(
        &st.pool,
        &host,
        input.node_id,
        input.proxied,
        input.notes.trim(),
    )
    .await?;
    audit(
        &st,
        &identity,
        "create",
        "domain",
        Some(domain.id),
        json!({"host": host}),
    )
    .await;
    Ok(Json(domain))
}

async fn update_domain(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateManagedDomain>,
) -> Result<Json<ManagedDomain>, ApiError> {
    if let Patch::Value(node_id) = input.node_id {
        if repo::get_node(&st.pool, node_id).await?.is_none() {
            return Err(ApiError::bad_request("node_id does not exist"));
        }
    }
    let domain = repo::update_managed_domain(&st.pool, id, &input)
        .await?
        .ok_or_else(|| ApiError::not_found("domain not found"))?;
    audit(&st, &identity, "update", "domain", Some(id), json!({})).await;
    Ok(Json(domain))
}

async fn delete_domain(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_managed_domain(&st.pool, id).await? {
        return Err(ApiError::not_found("domain not found"));
    }
    audit(&st, &identity, "delete", "domain", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// resolve DNS, probe :443 reachability + cert expiry, and store the verdict.
async fn verify_domain(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ManagedDomain>, ApiError> {
    crate::domains::run_and_store(&st.pool, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("domain not found"))
}

// --- routing profiles -------------------------------------------------------

fn valid_geo_codes(codes: &[String]) -> bool {
    codes.iter().all(|c| {
        !c.is_empty()
            && c.len() <= 32
            && c.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

async fn list_profiles(State(st): State<AppState>) -> Result<Json<Vec<RoutingProfile>>, ApiError> {
    Ok(Json(repo::list_routing_profiles(&st.pool).await?))
}

async fn create_profile(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewRoutingProfile>,
) -> Result<Json<RoutingProfile>, ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    if !valid_geo_codes(&input.direct_geosite) || !valid_geo_codes(&input.direct_geoip) {
        return Err(ApiError::bad_request(
            "geosite/geoip codes must be lowercase (e.g. cn, ru, category-ads-all)",
        ));
    }
    let profile = repo::create_routing_profile(&st.pool, &input).await?;
    audit(
        &st,
        &identity,
        "create",
        "routing_profile",
        Some(profile.id),
        json!({"name": profile.name}),
    )
    .await;
    Ok(Json(profile))
}

async fn update_profile(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateRoutingProfile>,
) -> Result<Json<RoutingProfile>, ApiError> {
    if let Some(codes) = &input.direct_geosite {
        if !valid_geo_codes(codes) {
            return Err(ApiError::bad_request("invalid geosite codes"));
        }
    }
    if let Some(codes) = &input.direct_geoip {
        if !valid_geo_codes(codes) {
            return Err(ApiError::bad_request("invalid geoip codes"));
        }
    }
    let profile = repo::update_routing_profile(&st.pool, id, &input)
        .await?
        .ok_or_else(|| ApiError::not_found("routing profile not found"))?;
    audit(
        &st,
        &identity,
        "update",
        "routing_profile",
        Some(id),
        json!({}),
    )
    .await;
    Ok(Json(profile))
}

async fn delete_profile(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_routing_profile(&st.pool, id).await? {
        return Err(ApiError::not_found("routing profile not found"));
    }
    audit(
        &st,
        &identity,
        "delete",
        "routing_profile",
        Some(id),
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AssignProfile {
    #[serde(default)]
    profile_id: Option<Uuid>,
}

async fn assign_profile(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(user_id): Path<Uuid>,
    Json(input): Json<AssignProfile>,
) -> Result<StatusCode, ApiError> {
    if let Some(pid) = input.profile_id {
        if repo::get_routing_profile(&st.pool, pid).await?.is_none() {
            return Err(ApiError::bad_request("routing profile does not exist"));
        }
    }
    if !repo::set_user_routing_profile(&st.pool, user_id, input.profile_id).await? {
        return Err(ApiError::not_found("user not found"));
    }
    // routing rules ride in the subscription; re-pushing nodes is not required.
    audit(
        &st,
        &identity,
        "assign_routing_profile",
        "user",
        Some(user_id),
        json!({"profile_id": input.profile_id}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn node_pushes(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NodePushEvent>>, ApiError> {
    Ok(Json(repo::list_node_pushes(&st.pool, id, 100).await?))
}

async fn node_enrollments(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<EnrollmentToken>>, ApiError> {
    Ok(Json(repo::list_enrollment_tokens(&st.pool, id).await?))
}

#[derive(Deserialize)]
struct CreateEnrollmentInput {
    #[serde(default = "default_enrollment_minutes")]
    expires_in_minutes: i64,
}

fn default_enrollment_minutes() -> i64 {
    30
}

async fn create_enrollment(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<CreateEnrollmentInput>,
) -> Result<Json<JsonValue>, ApiError> {
    if !(5..=1440).contains(&input.expires_in_minutes) {
        return Err(ApiError::bad_request(
            "expires_in_minutes must be between 5 and 1440",
        ));
    }
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let token = auth::random_token()?;
    let expires_at = Utc::now() + Duration::minutes(input.expires_in_minutes);
    let enrollment = repo::create_enrollment_token(
        &st.pool,
        id,
        &auth::token_hash(&token),
        identity.admin_id,
        expires_at,
    )
    .await?;
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("127.0.0.1:8080");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("https");
    let master = format!("{scheme}://{host}");
    audit(
        &st,
        &identity,
        "create",
        "enrollment",
        Some(enrollment.id),
        json!({"node_id": id}),
    )
    .await;
    tracing::info!(code = "M0801", node = %id, "minted an enrollment token");
    Ok(Json(json!({
        "id": enrollment.id,
        "node_id": id,
        "node": node.name,
        "token": token,
        "expires_at": expires_at,
        "install_command": format!("sudo -u honey /opt/honey/bin/honey-enroll --master {master} --token {token}")
    })))
}

#[derive(Deserialize)]
struct ClaimEnrollmentInput {
    csr_pem: String,
}

async fn claim_enrollment(
    State(st): State<AppState>,
    Path(token): Path<String>,
    Json(input): Json<ClaimEnrollmentInput>,
) -> Result<Json<JsonValue>, ApiError> {
    if token.len() < 32 || token.len() > 128 {
        return Err(ApiError::not_found("enrollment token not found"));
    }
    let node_id = repo::claim_enrollment_token(&st.pool, &auth::token_hash(&token))
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                code = "M0803",
                "rejected an enrollment claim: invalid, expired or already used"
            );
            ApiError::not_found("enrollment token is invalid, expired, or already used")
        })?;
    let node = repo::get_node(&st.pool, node_id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let issued = crate::pki::issue_node_certificate(&st.certs_dir, node_id, &input.csr_pem)
        .await
        .map_err(|error| {
            tracing::error!(%error, node = %node_id, "certificate issuance failed");
            ApiError::internal("certificate issuance failed")
        })?;
    repo::add_node_certificate(
        &st.pool,
        node_id,
        &issued.serial_number,
        &issued.fingerprint_sha256,
        &issued.subject,
        issued.not_before,
        issued.not_after,
    )
    .await?;
    let tls_server_name = format!("node-{node_id}.honey");
    repo::set_node_tls_name(&st.pool, node_id, &tls_server_name).await?;
    repo::record_audit(
        &st.pool,
        None,
        Some("node-enrollment"),
        "claim",
        "node_certificate",
        Some(&node_id.to_string()),
        None,
        json!({"serial": issued.serial_number, "fingerprint_sha256": issued.fingerprint_sha256}),
    )
    .await?;
    tracing::info!(code = "M0802", node = %node_id, "node enrolled, cert issued");
    Ok(Json(json!({
        "node_id": node_id,
        "node_name": node.name,
        "transport": node.transport,
        "tls_server_name": tls_server_name,
        "certificate_pem": issued.certificate_pem,
        "ca_pem": issued.ca_pem,
        "serial_number": issued.serial_number,
        "fingerprint_sha256": issued.fingerprint_sha256,
        "expires_at": issued.not_after
    })))
}

async fn revoke_enrollment(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::revoke_enrollment_token(&st.pool, id).await? {
        return Err(ApiError::not_found("active enrollment token not found"));
    }
    audit(&st, &identity, "revoke", "enrollment", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn node_certificates(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NodeCertificate>>, ApiError> {
    Ok(Json(repo::list_node_certificates(&st.pool, id).await?))
}

async fn revoke_certificate(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let node_id = repo::revoke_node_certificate(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("active certificate not found"))?;
    // Existing channels are not grandfathered: the next operation must make a
    // fresh TLS connection and pass the authoritative fingerprint inventory.
    st.registry.remove(node_id).await;
    audit(
        &st,
        &identity,
        "revoke",
        "node_certificate",
        Some(id),
        json!({"node_id": node_id}),
    )
    .await;
    tracing::warn!(code = "M0809", %node_id, certificate_id = %id, "node certificate revoked and live channel evicted");
    Ok(StatusCode::NO_CONTENT)
}

async fn audit(
    st: &AppState,
    identity: &Identity,
    action: &str,
    resource_type: &str,
    resource_id: Option<Uuid>,
    details: JsonValue,
) {
    if let Err(error) = repo::record_audit(
        &st.pool,
        identity.admin_id,
        Some(&identity.username),
        action,
        resource_type,
        resource_id.as_ref().map(Uuid::to_string).as_deref(),
        None,
        details,
    )
    .await
    {
        tracing::error!(%error, "could not persist audit event");
    }
}

async fn list_nodes(State(st): State<AppState>) -> Result<Json<Vec<Node>>, ApiError> {
    Ok(Json(repo::list_nodes(&st.pool).await?))
}

async fn create_node(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewNode>,
) -> Result<Json<Node>, ApiError> {
    validate_node(&input)?;
    let node = repo::create_node(&st.pool, input).await?;
    capture_version(&st, "node", node.id, &node, &identity).await;
    audit(
        &st,
        &identity,
        "create",
        "node",
        Some(node.id),
        json!({"name": node.name}),
    )
    .await;
    Ok(Json(node))
}

async fn get_node(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Node>, ApiError> {
    Ok(Json(
        repo::get_node(&st.pool, id)
            .await?
            .ok_or_else(|| ApiError::not_found("node not found"))?,
    ))
}

async fn update_node(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateNode>,
) -> Result<Json<Node>, ApiError> {
    validate_update_node(&input)?;
    let node = repo::update_node(&st.pool, id, input)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    push_nodes(&st, [id]).await;
    capture_version(&st, "node", id, &node, &identity).await;
    audit(
        &st,
        &identity,
        "update",
        "node",
        Some(id),
        json!({"name": node.name}),
    )
    .await;
    Ok(Json(node))
}

async fn set_node_labels(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<SetLabels>,
) -> Result<Json<Node>, ApiError> {
    let labels = normalize_labels(input.labels)?;
    let node = repo::set_node_labels(&st.pool, id, &labels)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    audit(
        &st,
        &identity,
        "labels",
        "node",
        Some(id),
        json!({"labels": labels}),
    )
    .await;
    Ok(Json(node))
}

async fn delete_node(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_node(&st.pool, id).await? {
        return Err(ApiError::not_found("node not found"));
    }
    st.registry.remove(id).await;
    audit(&st, &identity, "delete", "node", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct PreflightReport {
    ok: bool,
    gate: String,
    targets: Vec<crate::reach::PreflightTarget>,
}

/// Pre-rollout signal: probe the node's control port and its inbounds' public
/// ports. An open port only proves reachability from the master's network — it
/// is not a "clean IP" guarantee (blocklist/reputation checks are separate).
async fn node_preflight(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PreflightReport>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let targets = crate::reach::preflight(&st.pool, &node).await?;
    Ok(Json(PreflightReport {
        ok: crate::reach::failures(&targets).is_empty(),
        gate: preflight_gate(&st.pool).await,
        targets,
    }))
}

#[derive(Deserialize)]
struct BenchmarkQuery {
    mb: Option<f64>,
}

#[derive(Serialize)]
struct BenchmarkReport {
    size_mb: f64,
    latency_ms: f64,
    up_mbps: f64,
    down_mbps: f64,
}

/// Coarse master↔node throughput over the mTLS control channel. This measures
/// the control path, not the data plane, and each leg is capped by the gRPC
/// message limit — treat it as a capacity/quality signal, not a line-rate test.
async fn node_benchmark(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Query(query): Query<BenchmarkQuery>,
) -> Result<Json<BenchmarkReport>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if !st.registry.is_connected(id).await && node.transport != "dial" {
        st.registry
            .connect_serve(&node, &st.certs_dir)
            .await
            .map_err(|error| ApiError::bad_gateway(format!("agent connection failed: {error}")))?;
    }
    // 4 MiB is the default gRPC message ceiling; stay under it.
    let size_mb = query.mb.unwrap_or(2.0).clamp(0.25, 3.0);
    let bytes = (size_mb * 1024.0 * 1024.0) as usize;
    let (latency_ms, up_mbps, down_mbps) = st
        .registry
        .benchmark(id, bytes)
        .await
        .map_err(|error| ApiError::bad_gateway(format!("benchmark failed: {error}")))?;
    audit(
        &st,
        &identity,
        "benchmark",
        "node",
        Some(id),
        json!({"size_mb": size_mb}),
    )
    .await;
    Ok(Json(BenchmarkReport {
        size_mb,
        latency_ms,
        up_mbps,
        down_mbps,
    }))
}

/// Runtime gate mode: `off` | `warn` (default) | `block`.
async fn preflight_gate(pool: &PgPool) -> String {
    match repo::get_setting(pool, "preflight_gate").await {
        Ok(Some(v)) if matches!(v.trim(), "off" | "warn" | "block") => v.trim().to_string(),
        _ => "warn".to_string(),
    }
}

async fn push_node(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<PushResult>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;

    // pre-rollout gate: probe before touching the node's running config.
    let gate = preflight_gate(&st.pool).await;
    if gate != "off" {
        let targets = crate::reach::preflight(&st.pool, &node).await?;
        let failed = crate::reach::failures(&targets);
        if !failed.is_empty() {
            let summary = failed
                .iter()
                .map(|t| format!("{} ({})", t.label, t.target))
                .collect::<Vec<_>>()
                .join(", ");
            if gate == "block" {
                return Err(ApiError::bad_request(format!(
                    "preflight gate blocked the rollout — unreachable: {summary}. Fix the endpoint or set the preflight gate to warn/off."
                )));
            }
            tracing::warn!(
                code = "M1504",
                node = %node.name,
                "preflight warning before push — unreachable: {summary}"
            );
        }
    }

    if !st.registry.is_connected(id).await && node.transport != "dial" {
        st.registry
            .connect_serve(&node, &st.certs_dir)
            .await
            .map_err(|error| ApiError::bad_gateway(format!("agent connection failed: {error}")))?;
    }
    let status = st
        .registry
        .push_with_context(id, "api", identity.admin_id)
        .await
        .map_err(|error| ApiError::bad_gateway(format!("agent rejected config: {error}")))?;
    audit(
        &st,
        &identity,
        "push",
        "node",
        Some(id),
        json!({"state": format!("{:?}", status.state())}),
    )
    .await;
    Ok(Json(PushResult {
        node: node.name,
        state: format!("{:?}", status.state()),
        pid: status.pid,
        message: status.message,
    }))
}

async fn node_config_preview(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::spec::ConfigPreview>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let candidate = crate::spec::build_node_spec(&st.pool, id).await?;
    Ok(Json(crate::spec::preview(
        &candidate,
        node.applied_spec_hash,
        node.applied_spec_summary,
    )))
}

#[derive(Serialize)]
struct CoreDriftView {
    core: String,
    drifted: bool,
    running_present: bool,
}

#[derive(Serialize)]
struct DriftView {
    pending_push: bool,
    drifted: bool,
    cores: Vec<CoreDriftView>,
}

/// Compare the node's on-disk config against what the current spec would build.
/// `pending_push` distinguishes "desired changed but not pushed" from real drift
/// (running config edited / half-applied while the spec is already applied).
async fn node_config_drift(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DriftView>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if !st.registry.is_connected(id).await && node.transport != "dial" {
        st.registry
            .connect_serve(&node, &st.certs_dir)
            .await
            .map_err(|error| ApiError::bad_gateway(format!("agent connection failed: {error}")))?;
    }
    let candidate = crate::spec::build_node_spec(&st.pool, id).await?;
    let preview = crate::spec::preview(
        &candidate,
        node.applied_spec_hash,
        node.applied_spec_summary,
    );
    let cores = st
        .registry
        .config_drift(id, candidate)
        .await
        .map_err(|error| ApiError::bad_gateway(format!("agent config-drift failed: {error}")))?;
    let drifted = cores.iter().any(|c| c.drifted);
    Ok(Json(DriftView {
        pending_push: preview.changed,
        drifted,
        cores: cores
            .into_iter()
            .map(|c| CoreDriftView {
                core: if c.core == crate::pb::CoreKind::Xray as i32 {
                    "xray".into()
                } else {
                    "singbox".into()
                },
                drifted: c.drifted,
                running_present: c.running_present,
            })
            .collect(),
    }))
}

#[derive(Serialize)]
struct NodeMetricsView {
    supported: bool,
    cpu_percent: f64,
    cpu_cores: u32,
    mem_total: u64,
    mem_used: u64,
    disk_total: u64,
    disk_used: u64,
    net_rx_speed: u64,
    net_tx_speed: u64,
    load1: f64,
    uptime_secs: i64,
}

/// Live host metrics for one node, read on demand from the agent.
async fn node_metrics(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeMetricsView>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if !st.registry.is_connected(id).await && node.transport != "dial" {
        st.registry
            .connect_serve(&node, &st.certs_dir)
            .await
            .map_err(|error| ApiError::bad_gateway(format!("agent connection failed: {error}")))?;
    }
    let m = st
        .registry
        .metrics(id)
        .await
        .map_err(|error| ApiError::bad_gateway(format!("agent metrics failed: {error}")))?;
    Ok(Json(NodeMetricsView {
        supported: m.supported,
        cpu_percent: m.cpu_percent,
        cpu_cores: m.cpu_cores,
        mem_total: m.mem_total,
        mem_used: m.mem_used,
        disk_total: m.disk_total,
        disk_used: m.disk_used,
        net_rx_speed: m.net_rx_speed,
        net_tx_speed: m.net_tx_speed,
        load1: m.load1,
        uptime_secs: m.uptime_secs,
    }))
}

async fn dry_run_node(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<PushResult>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if !st.registry.is_connected(id).await && node.transport != "dial" {
        st.registry
            .connect_serve(&node, &st.certs_dir)
            .await
            .map_err(|error| ApiError::bad_gateway(format!("agent connection failed: {error}")))?;
    }
    let status = st
        .registry
        .dry_run(id)
        .await
        .map_err(|error| ApiError::bad_gateway(format!("agent validation failed: {error}")))?;
    audit(
        &st,
        &identity,
        "dry_run",
        "node",
        Some(id),
        json!({"state": format!("{:?}", status.state())}),
    )
    .await;
    Ok(Json(PushResult {
        node: node.name,
        state: format!("{:?}", status.state()),
        pid: 0,
        message: status.message,
    }))
}

#[derive(Serialize)]
struct PushResult {
    node: String,
    state: String,
    pid: i32,
    message: String,
}

#[derive(Default, Deserialize)]
struct AgentLogsQuery {
    #[serde(default)]
    after_seq: u64,
    #[serde(default = "default_agent_log_limit")]
    limit: u32,
}

fn default_agent_log_limit() -> u32 {
    200
}

#[derive(Serialize)]
struct AgentLogView {
    seq: u64,
    at_unix_ms: i64,
    level: String,
    code: String,
    message: String,
}

async fn node_agent_logs(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<AgentLogsQuery>,
) -> Result<Json<Vec<AgentLogView>>, ApiError> {
    let node = repo::get_node(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if !st.registry.is_connected(id).await && node.transport != "dial" {
        st.registry
            .connect_serve(&node, &st.certs_dir)
            .await
            .map_err(|error| ApiError::bad_gateway(format!("agent connection failed: {error}")))?;
    }
    let entries = st
        .registry
        .agent_logs(id, query.after_seq, query.limit.clamp(1, 500))
        .await
        .map_err(|error| ApiError::bad_gateway(format!("agent logs failed: {error}")))?;
    Ok(Json(
        entries
            .into_iter()
            .map(|entry| AgentLogView {
                seq: entry.seq,
                at_unix_ms: entry.at_unix_ms,
                level: entry.level,
                code: entry.code,
                message: entry.message,
            })
            .collect(),
    ))
}

async fn node_inbounds(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<InboundView>>, ApiError> {
    if repo::get_node(&st.pool, id).await?.is_none() {
        return Err(ApiError::not_found("node not found"));
    }
    Ok(Json(
        repo::node_inbounds(&st.pool, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

async fn get_inbound(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<InboundView>, ApiError> {
    Ok(Json(
        repo::get_inbound(&st.pool, id)
            .await?
            .ok_or_else(|| ApiError::not_found("inbound not found"))?
            .into(),
    ))
}

/// Public/admin API view of an inbound. Runtime-only private material stays in
/// the database/spec builder, while certificate state is explicit and stable
/// across UI and API clients.
#[derive(Serialize)]
struct InboundView {
    #[serde(flatten)]
    inbound: Inbound,
    certificate_source: &'static str,
    certificate_status: &'static str,
}

impl From<Inbound> for InboundView {
    fn from(inbound: Inbound) -> Self {
        let acme = inbound
            .extra
            .get("acme")
            .is_some_and(|value| !value.is_null());
        let (certificate_source, certificate_status) = if inbound.reality {
            ("reality", "not_applicable")
        } else if acme {
            ("acme", "managed")
        } else if inbound.tls_enabled {
            let configured = inbound.cert_path.as_deref().is_some_and(|v| !v.is_empty())
                && inbound.key_path.as_deref().is_some_and(|v| !v.is_empty());
            ("manual", if configured { "configured" } else { "missing" })
        } else {
            ("none", "not_applicable")
        };
        Self {
            inbound,
            certificate_source,
            certificate_status,
        }
    }
}

/// one-click REALITY x25519 keypair + short id for the inbound wizard.
async fn reality_keygen() -> Result<Json<JsonValue>, ApiError> {
    let kp = crate::reality::generate().map_err(|_| ApiError::internal("reality keygen failed"))?;
    Ok(Json(json!({
        "private_key": kp.private_key,
        "public_key": kp.public_key,
        "short_id": kp.short_id,
    })))
}

// --- reachability -----------------------------------------------------------

/// probe an inbound's public port from the master now and store the verdict.
async fn probe_inbound(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<InboundView>, ApiError> {
    let inbound = repo::get_inbound(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("inbound not found"))?;
    let node = repo::get_node(&st.pool, inbound.node_id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    crate::reach::check_one(
        &st.pool,
        id,
        &node.address,
        inbound.listen_port,
        &inbound.kind,
    )
    .await
    .map_err(|_| ApiError::internal("probe failed"))?;
    repo::get_inbound(&st.pool, id)
        .await?
        .map(InboundView::from)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("inbound not found"))
}

#[derive(Deserialize)]
struct ReachReport {
    reachable: bool,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    latency_ms: Option<i32>,
    #[serde(default)]
    error: Option<String>,
}

/// A vantage-point checker reports reachability from the target region. Verdicts
/// are logged and combined into a consensus; a fresh block auto-rotates the SNI
/// when the inbound has a pool, and re-pushes the node.
async fn report_reachability(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<ReachReport>,
) -> Result<StatusCode, ApiError> {
    let source = input
        .source
        .as_deref()
        .map(|s| bounded_auth_text(s, 64, "vantage"))
        .unwrap_or_else(|| "vantage".to_string());
    let verdict = repo::record_reachability_report(
        &st.pool,
        id,
        &source,
        input.reachable,
        input.latency_ms,
        input.error.as_deref(),
    )
    .await?
    .ok_or_else(|| ApiError::not_found("inbound not found"))?;
    // reactive safe SNI rotation: consensus just turned blocked → rotate to the
    // next owned SNI (if any) and re-push so clients pick up the change.
    if verdict == Some(false) {
        if let Some((node_id, sni)) = repo::rotate_inbound_sni(&st.pool, id).await? {
            tracing::warn!(code = "M1503", %id, sni = %sni, "endpoint blocked; rotated SNI");
            push_nodes(&st, [node_id]).await;
            audit(
                &st,
                &identity,
                "rotate_sni",
                "inbound",
                Some(id),
                json!({"reason": "auto", "sni": sni}),
            )
            .await;
        }
    }
    audit(
        &st,
        &identity,
        "report_reachability",
        "inbound",
        Some(id),
        json!({"reachable": input.reachable, "source": source}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_reachability(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<crate::db::models::ReachabilityReport>>, ApiError> {
    Ok(Json(
        repo::recent_reachability_reports(&st.pool, id, 30).await?,
    ))
}

async fn rotate_sni(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    match repo::rotate_inbound_sni(&st.pool, id).await? {
        Some((node_id, sni)) => {
            push_nodes(&st, [node_id]).await;
            audit(
                &st,
                &identity,
                "rotate_sni",
                "inbound",
                Some(id),
                json!({"reason": "manual", "sni": sni}),
            )
            .await;
            Ok(Json(json!({"server_name": sni})))
        }
        None => Err(ApiError::bad_request(
            "no alternate SNI in the pool to rotate to",
        )),
    }
}

// --- notification channels --------------------------------------------------

async fn list_channels(State(st): State<AppState>) -> Result<Json<Vec<NotifyChannel>>, ApiError> {
    Ok(Json(repo::list_notify_channels(&st.pool).await?))
}

async fn create_channel(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewNotifyChannel>,
) -> Result<Json<NotifyChannel>, ApiError> {
    if !matches!(
        input.kind.trim(),
        "webhook" | "discord" | "slack" | "telegram" | "email" | "sms" | "alertmanager"
    ) {
        return Err(ApiError::bad_request(
            "kind must be webhook, discord, slack, telegram, email, sms or alertmanager",
        ));
    }
    if input.name.trim().is_empty() || input.target.trim().is_empty() {
        return Err(ApiError::bad_request("name and target are required"));
    }
    let channel = repo::create_notify_channel(&st.pool, &input).await?;
    audit(
        &st,
        &identity,
        "create",
        "notify_channel",
        Some(channel.id),
        json!({"kind": channel.kind}),
    )
    .await;
    Ok(Json(channel))
}

async fn update_channel(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateNotifyChannel>,
) -> Result<Json<NotifyChannel>, ApiError> {
    let channel = repo::update_notify_channel(&st.pool, id, &input)
        .await?
        .ok_or_else(|| ApiError::not_found("channel not found"))?;
    audit(
        &st,
        &identity,
        "update",
        "notify_channel",
        Some(id),
        json!({}),
    )
    .await;
    Ok(Json(channel))
}

async fn delete_channel(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_notify_channel(&st.pool, id).await? {
        return Err(ApiError::not_found("channel not found"));
    }
    audit(
        &st,
        &identity,
        "delete",
        "notify_channel",
        Some(id),
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_channel(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let channel = repo::get_notify_channel(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("channel not found"))?;
    crate::notify::send(&channel, "✅ honey test", "notifications are working")
        .await
        .map_err(|error| ApiError::bad_gateway(format!("notification test failed: {error}")))?;
    Ok(StatusCode::NO_CONTENT)
}

// --- telegram chat allowlist ------------------------------------------------

async fn list_tg_chats(State(st): State<AppState>) -> Result<Json<Vec<TelegramChat>>, ApiError> {
    Ok(Json(repo::list_telegram_chats(&st.pool).await?))
}

fn default_tg_role() -> String {
    "user".to_string()
}

#[derive(Deserialize)]
struct NewTgChat {
    chat_id: i64,
    #[serde(default = "default_tg_role")]
    role: String,
    #[serde(default)]
    note: String,
}

async fn add_tg_chat(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewTgChat>,
) -> Result<Json<TelegramChat>, ApiError> {
    if !matches!(input.role.as_str(), "admin" | "user") {
        return Err(ApiError::bad_request("role must be admin or user"));
    }
    let chat =
        repo::add_telegram_chat(&st.pool, input.chat_id, &input.role, input.note.trim()).await?;
    audit(
        &st,
        &identity,
        "create",
        "telegram_chat",
        None,
        json!({"chat_id": input.chat_id, "role": input.role}),
    )
    .await;
    Ok(Json(chat))
}

async fn delete_tg_chat(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(chat_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_telegram_chat(&st.pool, chat_id).await? {
        return Err(ApiError::not_found("chat not found"));
    }
    audit(
        &st,
        &identity,
        "delete",
        "telegram_chat",
        None,
        json!({"chat_id": chat_id}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_inbound(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewInbound>,
) -> Result<Json<InboundView>, ApiError> {
    validate_inbound(&input)?;
    validate_upstream(&st.pool, input.upstream_inbound_id, None, &input.core).await?;
    let inbound = repo::create_inbound(&st.pool, input).await?;
    push_nodes(&st, [inbound.node_id]).await;
    capture_version(&st, "inbound", inbound.id, &inbound, &identity).await;
    audit(
        &st,
        &identity,
        "create",
        "inbound",
        Some(inbound.id),
        json!({"node_id": inbound.node_id, "protocol": inbound.kind}),
    )
    .await;
    Ok(Json(inbound.into()))
}

// --- managed external services (MTProto / NaiveProxy) -----------------------

const SERVICE_KINDS: &[&str] = &["mtproto", "naive"];

/// A safe hostname/identifier for values that land verbatim in a node-side
/// config file (mtg toml / Caddyfile): letters, digits, dot, dash, underscore.
fn safe_config_value(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn validate_service_config(kind: &str, config: &JsonValue) -> Result<(), ApiError> {
    let field = |name: &str| config.get(name).and_then(JsonValue::as_str).unwrap_or("");
    match kind {
        "mtproto" => {
            let host = field("host");
            if !host.is_empty() && !safe_config_value(host, 253) {
                return Err(ApiError::bad_request(
                    "fake-TLS host must be a bare hostname (letters, digits, . - _)",
                ));
            }
            if let Some(c) = config.get("concurrency").and_then(JsonValue::as_i64) {
                if !(0..=1_000_000).contains(&c) {
                    return Err(ApiError::bad_request(
                        "concurrency out of range (0..1000000)",
                    ));
                }
            }
            if let Some(p) = config
                .get("domain_fronting_port")
                .and_then(JsonValue::as_i64)
            {
                if p != 0 && !(1..=65_535).contains(&p) {
                    return Err(ApiError::bad_request("domain_fronting_port out of range"));
                }
            }
            let prefer = field("prefer_ip");
            if !prefer.is_empty()
                && !matches!(
                    prefer,
                    "prefer-ipv4" | "prefer-ipv6" | "only-ipv4" | "only-ipv6"
                )
            {
                return Err(ApiError::bad_request(
                    "prefer_ip must be prefer-ipv4|prefer-ipv6|only-ipv4|only-ipv6",
                ));
            }
        }
        "naive" => {
            let user = field("username");
            if !user.is_empty() && !safe_config_value(user, 64) {
                return Err(ApiError::bad_request(
                    "username must be letters, digits, . - _",
                ));
            }
            let domain = field("domain");
            if !domain.is_empty() && !safe_config_value(domain, 253) {
                return Err(ApiError::bad_request("TLS domain must be a bare hostname"));
            }
        }
        _ => {}
    }
    Ok(())
}

async fn list_services(
    State(st): State<AppState>,
    Path(node_id): Path<Uuid>,
) -> Result<Json<Vec<crate::db::models::NodeService>>, ApiError> {
    Ok(Json(repo::list_node_services(&st.pool, node_id).await?))
}

async fn create_service(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(node_id): Path<Uuid>,
    Json(mut input): Json<crate::db::models::NewNodeService>,
) -> Result<Json<crate::db::models::NodeService>, ApiError> {
    input.node_id = node_id;
    if !SERVICE_KINDS.contains(&input.kind.as_str()) {
        return Err(ApiError::bad_request("kind must be mtproto or naive"));
    }
    if input.name.trim().is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    if !(1..=65_535).contains(&input.listen_port) {
        return Err(ApiError::bad_request("listen_port out of range"));
    }
    // the agent writes these verbatim into an mtg toml / Caddyfile, so reject
    // anything that could inject a config directive (newlines, braces, quotes).
    validate_service_config(&input.kind, &input.config)?;
    repo::get_node(&st.pool, node_id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let svc = repo::create_node_service(&st.pool, input).await?;
    push_nodes(&st, [node_id]).await;
    audit(
        &st,
        &identity,
        "create",
        "service",
        Some(svc.id),
        json!({"node_id": node_id, "kind": svc.kind}),
    )
    .await;
    Ok(Json(svc))
}

async fn update_service(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<crate::db::models::UpdateNodeService>,
) -> Result<Json<crate::db::models::NodeService>, ApiError> {
    if let Some(config) = &input.config {
        let current = repo::get_node_service(&st.pool, id)
            .await?
            .ok_or_else(|| ApiError::not_found("service not found"))?;
        validate_service_config(&current.kind, config)?;
    }
    if let Some(port) = input.listen_port {
        if !(1..=65_535).contains(&port) {
            return Err(ApiError::bad_request("listen_port out of range"));
        }
    }
    let svc = repo::update_node_service(&st.pool, id, input)
        .await?
        .ok_or_else(|| ApiError::not_found("service not found"))?;
    push_nodes(&st, [svc.node_id]).await;
    audit(&st, &identity, "update", "service", Some(svc.id), json!({})).await;
    Ok(Json(svc))
}

async fn delete_service(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let svc = repo::get_node_service(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("service not found"))?;
    repo::delete_node_service(&st.pool, id).await?;
    push_nodes(&st, [svc.node_id]).await;
    audit(&st, &identity, "delete", "service", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- WireGuard / AmneziaWG interfaces ---------------------------------------

async fn list_wg(
    State(st): State<AppState>,
    Path(node_id): Path<Uuid>,
) -> Result<Json<Vec<crate::db::models::WgInterface>>, ApiError> {
    Ok(Json(repo::list_wg_interfaces(&st.pool, node_id).await?))
}

async fn create_wg(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(node_id): Path<Uuid>,
    Json(mut input): Json<crate::db::models::NewWgInterface>,
) -> Result<Json<crate::db::models::WgInterface>, ApiError> {
    input.node_id = node_id;
    if input.name.trim().is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    if !(1..=65_535).contains(&input.listen_port) {
        return Err(ApiError::bad_request("listen_port out of range"));
    }
    crate::wg::parse_cidr(&input.address_cidr)
        .map_err(|e| ApiError::bad_request(format!("address_cidr: {e}")))?;
    input.mtu = input.mtu.clamp(1280, 1500);
    repo::get_node(&st.pool, node_id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let iface = repo::create_wg_interface(&st.pool, input).await?;
    push_nodes(&st, [node_id]).await;
    audit(
        &st,
        &identity,
        "create",
        "wireguard",
        Some(iface.id),
        json!({"node_id": node_id, "name": iface.name, "amnezia": iface.amnezia}),
    )
    .await;
    Ok(Json(iface))
}

async fn update_wg(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<crate::db::models::UpdateWgInterface>,
) -> Result<Json<crate::db::models::WgInterface>, ApiError> {
    let iface = repo::update_wg_interface(&st.pool, id, input)
        .await?
        .ok_or_else(|| ApiError::not_found("wireguard interface not found"))?;
    push_nodes(&st, [iface.node_id]).await;
    audit(
        &st,
        &identity,
        "update",
        "wireguard",
        Some(iface.id),
        json!({}),
    )
    .await;
    Ok(Json(iface))
}

async fn delete_wg(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let iface = repo::get_wg_interface(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("wireguard interface not found"))?;
    repo::delete_wg_interface(&st.pool, id).await?;
    push_nodes(&st, [iface.node_id]).await;
    audit(&st, &identity, "delete", "wireguard", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_inbound(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateInbound>,
) -> Result<Json<InboundView>, ApiError> {
    validate_update_inbound(&input)?;
    let current = repo::get_inbound(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("inbound not found"))?;
    validate_effective_update(&current, &input)?;
    if let crate::db::models::Patch::Value(upstream) = input.upstream_inbound_id {
        let entry_core = input.core.as_deref().unwrap_or(&current.core);
        validate_upstream(&st.pool, Some(upstream), Some(id), entry_core).await?;
    }
    let inbound = repo::update_inbound(&st.pool, id, input)
        .await?
        .ok_or_else(|| ApiError::not_found("inbound not found"))?;
    push_nodes(&st, [inbound.node_id]).await;
    capture_version(&st, "inbound", id, &inbound, &identity).await;
    audit(
        &st,
        &identity,
        "update",
        "inbound",
        Some(id),
        json!({"node_id": inbound.node_id}),
    )
    .await;
    Ok(Json(inbound.into()))
}

async fn set_inbound_labels(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<SetLabels>,
) -> Result<Json<InboundView>, ApiError> {
    let labels = normalize_labels(input.labels)?;
    let inbound = repo::set_inbound_labels(&st.pool, id, &labels)
        .await?
        .ok_or_else(|| ApiError::not_found("inbound not found"))?;
    audit(
        &st,
        &identity,
        "labels",
        "inbound",
        Some(id),
        json!({"labels": labels}),
    )
    .await;
    Ok(Json(inbound.into()))
}

async fn delete_inbound(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let inbound = repo::get_inbound(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("inbound not found"))?;
    repo::delete_inbound(&st.pool, id).await?;
    push_nodes(&st, [inbound.node_id]).await;
    audit(
        &st,
        &identity,
        "delete",
        "inbound",
        Some(id),
        json!({"node_id": inbound.node_id}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// --- node groups (access model) --------------------------------------------

async fn list_groups(State(st): State<AppState>) -> Result<Json<Vec<NodeGroup>>, ApiError> {
    Ok(Json(repo::list_node_groups(&st.pool).await?))
}

async fn create_group(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewNodeGroup>,
) -> Result<Json<NodeGroup>, ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    let group = repo::create_node_group(&st.pool, &input).await?;
    audit(
        &st,
        &identity,
        "create",
        "node_group",
        Some(group.id),
        json!({"name": group.name}),
    )
    .await;
    Ok(Json(group))
}

async fn update_group(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateNodeGroup>,
) -> Result<Json<NodeGroup>, ApiError> {
    let group = repo::update_node_group(&st.pool, id, &input)
        .await?
        .ok_or_else(|| ApiError::not_found("group not found"))?;
    audit(&st, &identity, "update", "node_group", Some(id), json!({})).await;
    Ok(Json(group))
}

async fn delete_group(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !repo::delete_node_group(&st.pool, id).await? {
        return Err(ApiError::bad_request(
            "group not found, or it is the default group",
        ));
    }
    audit(&st, &identity, "delete", "node_group", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_node_groups(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Uuid>>, ApiError> {
    if repo::get_node(&st.pool, id).await?.is_none() {
        return Err(ApiError::not_found("node not found"));
    }
    Ok(Json(repo::node_group_ids(&st.pool, id).await?))
}

async fn set_node_groups(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<GroupIds>,
) -> Result<StatusCode, ApiError> {
    if repo::get_node(&st.pool, id).await?.is_none() {
        return Err(ApiError::not_found("node not found"));
    }
    repo::set_node_groups(&st.pool, id, &input.group_ids).await?;
    // membership changes who reaches this node → re-push it.
    push_nodes(&st, [id]).await;
    audit(
        &st,
        &identity,
        "set_groups",
        "node",
        Some(id),
        json!({"groups": input.group_ids.len()}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_user_groups(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Uuid>>, ApiError> {
    owned_user(&st, &identity, id).await?;
    Ok(Json(repo::user_group_ids(&st.pool, id).await?))
}

async fn set_user_groups(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<GroupIds>,
) -> Result<StatusCode, ApiError> {
    owned_user(&st, &identity, id).await?;
    // a reseller may only assign groups it is entitled to sell.
    if identity.is_reseller() {
        let allowed: std::collections::HashSet<Uuid> =
            repo::reseller_group_ids(&st.pool, identity.admin_id.unwrap_or_default())
                .await?
                .into_iter()
                .collect();
        if !input.group_ids.iter().all(|g| allowed.contains(g)) {
            return Err(ApiError::forbidden(
                "one or more groups are outside your entitlement",
            ));
        }
    }
    // re-push nodes the user reached before AND after the change.
    let mut nodes: std::collections::HashSet<Uuid> = repo::user_node_ids(&st.pool, id)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    repo::set_user_groups(&st.pool, id, &input.group_ids).await?;
    nodes.extend(repo::user_node_ids(&st.pool, id).await.unwrap_or_default());
    push_nodes(&st, nodes).await;
    audit(
        &st,
        &identity,
        "set_groups",
        "user",
        Some(id),
        json!({"groups": input.group_ids.len()}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct UserView {
    #[serde(flatten)]
    user: User,
    active: bool,
    suppressed_reason: Option<String>,
}

impl From<User> for UserView {
    fn from(user: User) -> Self {
        let reason = user.suppressed_reason().map(str::to_string);
        Self {
            active: reason.is_none(),
            suppressed_reason: reason,
            user,
        }
    }
}

/// For a reseller, fetch a user and confirm they own it; for other roles just
/// fetch. 404 if missing, 403 if a reseller reaches for someone else's user.
async fn owned_user(st: &AppState, identity: &Identity, id: Uuid) -> Result<User, ApiError> {
    let user = repo::get_user(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    if identity.is_reseller() && user.created_by != identity.admin_id {
        return Err(ApiError::forbidden("this user is outside your scope"));
    }
    Ok(user)
}

async fn list_users(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<UserView>>, ApiError> {
    let users = match identity.admin_id {
        Some(admin_id) if identity.is_reseller() => {
            repo::list_users_for_creator(&st.pool, admin_id).await?
        }
        _ => repo::list_users(&st.pool).await?,
    };
    Ok(Json(users.into_iter().map(Into::into).collect()))
}

#[derive(Serialize)]
struct HaInstanceView {
    instance_id: Uuid,
    hostname: String,
    version: String,
    started_at: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    leader: bool,
    self_instance: bool,
}

#[derive(Serialize)]
struct HaStatus {
    instance_id: Uuid,
    is_leader: bool,
    leader_id: Option<Uuid>,
    lease_expires_at: Option<DateTime<Utc>>,
    instances: Vec<HaInstanceView>,
}

async fn self_update_enabled(pool: &PgPool) -> bool {
    repo::setting_i64(pool, "self_update_enabled", 0).await != 0
}

/// Check GitHub for a newer master release. Read-only: downloads nothing.
async fn update_check() -> Result<Json<crate::update::UpdateStatus>, ApiError> {
    crate::update::check()
        .await
        .map(Json)
        .map_err(|error| ApiError::bad_gateway(format!("update check failed: {error}")))
}

#[derive(Serialize)]
struct UpdateApplyResult {
    staged_version: String,
    restart_required: bool,
}

/// Download, SHA-256-verify and stage the latest release over the running
/// binary. Owner-only, audited, and gated behind the `self_update_enabled`
/// setting. The staged binary runs after the supervisor restarts the process.
async fn update_apply(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<UpdateApplyResult>, ApiError> {
    if !self_update_enabled(&st.pool).await {
        return Err(ApiError::forbidden(
            "self-update is disabled; enable it in runtime settings first",
        ));
    }
    let staged = crate::update::apply()
        .await
        .map_err(|error| ApiError::bad_request(format!("self-update failed: {error}")))?;
    audit(
        &st,
        &identity,
        "self-update",
        "master",
        None,
        json!({"staged_version": staged, "from": crate::update::CURRENT_VERSION}),
    )
    .await;
    crate::update::schedule_restart();
    Ok(Json(UpdateApplyResult {
        staged_version: staged,
        restart_required: true,
    }))
}

/// Multi-master status: who is up and which instance currently holds the lease
/// that gates the singleton background loops.
async fn ha_status(State(st): State<AppState>) -> Result<Json<HaStatus>, ApiError> {
    let me = crate::ha::instance_id();
    let lease = repo::ha_leader(&st.pool).await?;
    let leader_id = lease.as_ref().map(|(holder, _)| *holder);
    let instances = repo::ha_instances(&st.pool)
        .await?
        .into_iter()
        .map(
            |(instance_id, hostname, version, started_at, last_seen)| HaInstanceView {
                leader: Some(instance_id) == leader_id,
                self_instance: instance_id == me,
                instance_id,
                hostname,
                version,
                started_at,
                last_seen,
            },
        )
        .collect();
    Ok(Json(HaStatus {
        instance_id: me,
        is_leader: crate::ha::is_leader(),
        leader_id,
        lease_expires_at: lease.map(|(_, expires)| expires),
        instances,
    }))
}

#[derive(Serialize)]
struct GeoBucket {
    code: String,
    connections: i64,
    users: i64,
    up_bytes: i64,
    down_bytes: i64,
}

#[derive(Serialize)]
struct GeoDistribution {
    /// how many country ranges the operator's table supplied (0 = only
    /// special-use ranges are known, so public IPs report `unknown`)
    country_ranges: usize,
    total_connections: i64,
    buckets: Vec<GeoBucket>,
}

/// Geographic distribution of the **current** live connections, bucketed by the
/// source address. This is a live snapshot (not a historical heatmap): source
/// IPs are only observable while a connection is open. Reseller-scoped.
async fn geo_distribution(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<GeoDistribution>, ApiError> {
    let conns = collect_live_connections(&st, &identity).await?;
    let mut by_code: std::collections::HashMap<
        String,
        (i64, i64, i64, std::collections::HashSet<String>),
    > = std::collections::HashMap::new();
    for c in &conns {
        let code = crate::geo::lookup_str(&c.source_ip).to_string();
        let entry = by_code
            .entry(code)
            .or_insert((0, 0, 0, std::collections::HashSet::new()));
        entry.0 += 1;
        entry.1 += c.up_bytes;
        entry.2 += c.down_bytes;
        if !c.user.is_empty() {
            entry.3.insert(c.user.clone());
        }
    }
    let mut buckets: Vec<GeoBucket> = by_code
        .into_iter()
        .map(|(code, (connections, up, down, users))| GeoBucket {
            code,
            connections,
            users: users.len() as i64,
            up_bytes: up,
            down_bytes: down,
        })
        .collect();
    buckets.sort_by(|a, b| b.connections.cmp(&a.connections).then(a.code.cmp(&b.code)));
    Ok(Json(GeoDistribution {
        country_ranges: crate::geo::country_ranges(),
        total_connections: conns.len() as i64,
        buckets,
    }))
}

#[derive(Serialize)]
struct LiveConnView {
    node_id: Uuid,
    node: String,
    user: String,
    user_id: Option<Uuid>,
    source_ip: String,
    destination: String,
    network: String,
    up_bytes: i64,
    down_bytes: i64,
    started_at: Option<i64>,
    chain: String,
}

/// Point-in-time active connections across every connected node, read from each
/// agent's Clash API. Resellers only see their own users' connections.
async fn live_connections(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
) -> Result<Json<Vec<LiveConnView>>, ApiError> {
    Ok(Json(collect_live_connections(&st, &identity).await?))
}

/// Shared gathering for the live view and the geo distribution.
async fn collect_live_connections(
    st: &AppState,
    identity: &Identity,
) -> Result<Vec<LiveConnView>, ApiError> {
    let users = repo::list_users(&st.pool).await?;
    let by_name: std::collections::HashMap<String, (Uuid, Option<Uuid>)> = users
        .iter()
        .map(|u| (u.username.clone(), (u.id, u.created_by)))
        .collect();
    let reseller = if identity.is_reseller() {
        identity.admin_id
    } else {
        None
    };
    let node_names: std::collections::HashMap<Uuid, String> = repo::list_nodes(&st.pool)
        .await?
        .into_iter()
        .map(|n| (n.id, n.name))
        .collect();

    let mut out = Vec::new();
    for node_id in st.registry.connected_ids().await {
        let conns = match st
            .registry
            .connections(node_id, crate::pb::CoreKind::Singbox)
            .await
        {
            Ok(conns) => conns,
            Err(_) => continue, // node dropped mid-scan; skip it
        };
        let node = node_names.get(&node_id).cloned().unwrap_or_default();
        for c in conns {
            let owner = by_name.get(&c.user);
            if let Some(rid) = reseller {
                match owner {
                    Some((_, Some(created_by))) if *created_by == rid => {}
                    _ => continue,
                }
            }
            out.push(LiveConnView {
                node_id,
                node: node.clone(),
                user_id: owner.map(|(id, _)| *id),
                user: c.user,
                source_ip: c.source_ip,
                destination: c.destination,
                network: c.network,
                up_bytes: c.up_bytes as i64,
                down_bytes: c.down_bytes as i64,
                started_at: (c.started_at > 0).then_some(c.started_at),
                chain: c.chain,
            });
        }
    }
    Ok(out)
}

async fn get_user(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserView>, ApiError> {
    Ok(Json(owned_user(&st, &identity, id).await?.into()))
}

#[derive(Serialize)]
struct CreatedUser {
    #[serde(flatten)]
    user: UserView,
    subscription_token: Uuid,
    subscription_path: String,
}

async fn create_user(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Json(input): Json<NewUser>,
) -> Result<Json<CreatedUser>, ApiError> {
    validate_user(&input)?;
    // reseller path: enforce allocation caps, stamp ownership, and grant the
    // reseller's own groups (never the default group).
    let reseller_groups = if identity.is_reseller() {
        let admin_id = identity
            .admin_id
            .ok_or_else(|| ApiError::forbidden("reseller identity has no account"))?;
        let admin = repo::get_admin(&st.pool, admin_id)
            .await?
            .ok_or_else(|| ApiError::forbidden("reseller account not found"))?;
        if admin.max_users > 0 {
            let owned = repo::count_users_for_creator(&st.pool, admin_id).await?;
            if owned >= admin.max_users as i64 {
                return Err(ApiError::forbidden(format!(
                    "user limit reached ({} of {})",
                    owned, admin.max_users
                )));
            }
        }
        if admin.user_traffic_ceiling_bytes > 0
            && (input.traffic_limit_bytes <= 0
                || input.traffic_limit_bytes > admin.user_traffic_ceiling_bytes)
        {
            return Err(ApiError::forbidden(format!(
                "per-user traffic limit must be between 1 and {} bytes",
                admin.user_traffic_ceiling_bytes
            )));
        }
        if admin.traffic_limit_bytes > 0 {
            let used = repo::reseller_traffic_used(&st.pool, admin_id).await?;
            if used >= admin.traffic_limit_bytes {
                return Err(ApiError::forbidden(
                    "your traffic budget is exhausted — ask an admin to raise it",
                ));
            }
        }
        let groups = repo::reseller_group_ids(&st.pool, admin_id).await?;
        if groups.is_empty() {
            return Err(ApiError::forbidden(
                "your account has no groups to sell yet — ask an admin",
            ));
        }
        Some((admin_id, groups))
    } else {
        None
    };

    let created_by = reseller_groups
        .as_ref()
        .map(|(id, _)| *id)
        .or(identity.admin_id);
    let grant_default = reseller_groups.is_none();
    let (user, token) = repo::create_user(&st.pool, input, created_by, grant_default).await?;
    if let Some((_, groups)) = &reseller_groups {
        repo::set_user_groups(&st.pool, user.id, groups).await?;
    }
    let nodes = repo::user_node_ids(&st.pool, user.id).await?;
    push_nodes(&st, nodes).await;
    capture_version(&st, "user", user.id, &user, &identity).await;
    audit(
        &st,
        &identity,
        "create",
        "user",
        Some(user.id),
        json!({"username": user.username}),
    )
    .await;
    Ok(Json(CreatedUser {
        user: user.into(),
        subscription_token: token,
        subscription_path: format!("/sub/{token}"),
    }))
}

async fn update_user(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateUser>,
) -> Result<Json<UserView>, ApiError> {
    validate_update_user(&input)?;
    owned_user(&st, &identity, id).await?;
    let nodes = repo::user_node_ids(&st.pool, id).await?;
    let user = repo::update_user(&st.pool, id, input)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    push_nodes(&st, nodes).await;
    capture_version(&st, "user", id, &user, &identity).await;
    audit(
        &st,
        &identity,
        "update",
        "user",
        Some(id),
        json!({"username": user.username}),
    )
    .await;
    Ok(Json(user.into()))
}

async fn set_user_labels(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<SetLabels>,
) -> Result<Json<UserView>, ApiError> {
    owned_user(&st, &identity, id).await?;
    let labels = normalize_labels(input.labels)?;
    let user = repo::set_user_labels(&st.pool, id, &labels)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    audit(
        &st,
        &identity,
        "labels",
        "user",
        Some(id),
        json!({"labels": labels}),
    )
    .await;
    Ok(Json(user.into()))
}

async fn delete_user(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    owned_user(&st, &identity, id).await?;
    let nodes = repo::user_node_ids(&st.pool, id).await?;
    if !repo::delete_user(&st.pool, id).await? {
        return Err(ApiError::not_found("user not found"));
    }
    push_nodes(&st, nodes).await;
    audit(&st, &identity, "delete", "user", Some(id), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct CredentialsResult {
    uuid: Uuid,
    password: String,
}

async fn rotate_user_credentials(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<RotateCredentials>,
) -> Result<Json<CredentialsResult>, ApiError> {
    if input.password.as_deref().is_some_and(str::is_empty) {
        return Err(ApiError::bad_request("password must not be empty"));
    }
    owned_user(&st, &identity, id).await?;
    let nodes = repo::user_node_ids(&st.pool, id).await?;
    let (uuid, password) = repo::rotate_credentials(&st.pool, id, input.password)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    push_nodes(&st, nodes).await;
    audit(
        &st,
        &identity,
        "rotate_credentials",
        "user",
        Some(id),
        json!({}),
    )
    .await;
    Ok(Json(CredentialsResult { uuid, password }))
}

#[derive(Serialize)]
struct SubscriptionTokenResult {
    subscription_token: Uuid,
    subscription_path: String,
}

async fn reveal_subscription(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<RevealedSubscription>, ApiError> {
    owned_user(&st, &identity, id).await?;
    match repo::reveal_subscription_token(&st.pool, id).await? {
        None => Err(ApiError::not_found("user not found")),
        Some(None) => Ok(Json(RevealedSubscription {
            subscription_token: None,
            subscription_path: None,
        })),
        Some(Some(token)) => Ok(Json(RevealedSubscription {
            subscription_path: Some(format!("/sub/{token}")),
            subscription_token: Some(token),
        })),
    }
}

// --- multi-subscription profiles (named links per user) ---------------------

async fn list_user_subscriptions(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<crate::db::models::UserSubscription>>, ApiError> {
    owned_user(&st, &identity, id).await?;
    Ok(Json(repo::list_user_subscriptions(&st.pool, id).await?))
}

#[derive(Deserialize)]
struct NewSubInput {
    name: String,
}

async fn create_user_subscription(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<NewSubInput>,
) -> Result<Json<JsonValue>, ApiError> {
    owned_user(&st, &identity, id).await?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 25 {
        return Err(ApiError::bad_request("name is required (max 25 chars)"));
    }
    let (sub, token) = repo::create_user_subscription(&st.pool, id, name).await?;
    audit(
        &st,
        &identity,
        "create",
        "user_subscription",
        Some(id),
        json!({"name": sub.name}),
    )
    .await;
    Ok(Json(json!({
        "id": sub.id, "name": sub.name,
        "subscription_token": token, "subscription_path": format!("/sub/{token}")
    })))
}

async fn reveal_user_subscription(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path((id, sid)): Path<(Uuid, Uuid)>,
) -> Result<Json<RevealedSubscription>, ApiError> {
    owned_user(&st, &identity, id).await?;
    match repo::reveal_user_subscription(&st.pool, id, sid).await? {
        None => Err(ApiError::not_found("subscription not found")),
        Some(None) => Ok(Json(RevealedSubscription {
            subscription_token: None,
            subscription_path: None,
        })),
        Some(Some(token)) => Ok(Json(RevealedSubscription {
            subscription_path: Some(format!("/sub/{token}")),
            subscription_token: Some(token),
        })),
    }
}

async fn delete_user_subscription(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path((id, sid)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    owned_user(&st, &identity, id).await?;
    if !repo::delete_user_subscription(&st.pool, id, sid).await? {
        return Err(ApiError::not_found("subscription not found"));
    }
    audit(
        &st,
        &identity,
        "delete",
        "user_subscription",
        Some(id),
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct RevealedSubscription {
    subscription_token: Option<String>,
    subscription_path: Option<String>,
}

async fn rotate_subscription(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<SubscriptionTokenResult>, ApiError> {
    owned_user(&st, &identity, id).await?;
    let token = repo::rotate_subscription_token(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    audit(
        &st,
        &identity,
        "rotate_subscription",
        "user",
        Some(id),
        json!({}),
    )
    .await;
    Ok(Json(SubscriptionTokenResult {
        subscription_token: token,
        subscription_path: format!("/sub/{token}"),
    }))
}

async fn reset_user_traffic(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserView>, ApiError> {
    owned_user(&st, &identity, id).await?;
    let nodes = repo::user_node_ids(&st.pool, id).await?;
    let user = repo::reset_user_traffic(&st.pool, id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    push_nodes(&st, nodes).await;
    audit(&st, &identity, "reset_traffic", "user", Some(id), json!({})).await;
    Ok(Json(user.into()))
}

#[derive(Serialize)]
struct SubscriptionDocument {
    id: Uuid,
    username: String,
    status: String,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    traffic_limit_bytes: i64,
    used_traffic_bytes: i64,
    links: Vec<EndpointLink>,
    singbox_config: JsonValue,
}

async fn subscription_document(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // a known VPN client fetching /sub/:token gets its tailored config directly —
    // one URL works in every app. browsers fall through to the dashboard / JSON.
    if let Some(fmt) = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .and_then(format_for_ua)
    {
        let (user, endpoints) = load_subscription(&st, token).await?;
        return tailored_response(&st, fmt, &user, &endpoints).await;
    }
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("application/json"));
    let (user, endpoints) = if wants_json {
        load_subscription(&st, token).await?
    } else {
        load_subscription_page(&st, token).await?
    };
    let links = subscription::endpoint_links(&user, &endpoints);
    if !wants_json {
        return Ok(crate::subscription_page::html_response(
            crate::subscription_page::render(&user, token, &links),
        ));
    }
    let profile = repo::routing_profile_for_user(&st.pool, user.id).await?;
    Ok(Json(SubscriptionDocument {
        id: user.id,
        username: user.username.clone(),
        status: user.suppressed_reason().unwrap_or("active").into(),
        created_at: user.created_at,
        expires_at: user.expires_at,
        traffic_limit_bytes: user.traffic_limit_bytes,
        used_traffic_bytes: user.used_traffic_bytes,
        links,
        singbox_config: subscription::singbox_client_config(&user, &endpoints, profile.as_ref()),
    })
    .into_response())
}

async fn subscription_links(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (user, endpoints) = load_subscription(&st, token).await?;
    let text = subscription::endpoint_links(&user, &endpoints)
        .into_iter()
        .filter_map(|link| link.uri)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let support = repo::all_settings(&st.pool)
        .await?
        .into_iter()
        .find(|(k, _)| k == "subscription_support_url")
        .map(|(_, v)| v);
    Ok((sub_headers(&user, support.as_deref()), text))
}

/// canonical client subscription: base64 links + Subscription-Userinfo header.
async fn subscription_v2ray(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let (user, endpoints) = load_subscription(&st, token).await?;
    let body = subscription::v2ray_document(&user, &endpoints);
    let support = repo::all_settings(&st.pool)
        .await?
        .into_iter()
        .find(|(k, _)| k == "subscription_support_url")
        .map(|(_, v)| v);
    Ok((sub_headers(&user, support.as_deref()), body))
}

#[derive(Deserialize)]
struct SpeedtestQuery {
    mb: Option<f64>,
}

/// Client-facing speed test payload. The client times this download to estimate
/// its own throughput. Note this measures **client ↔ master**, not client ↔ node:
/// agents speak only mTLS gRPC and expose no public HTTP endpoint to serve from.
async fn subscription_speedtest(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
    Query(query): Query<SpeedtestQuery>,
) -> Result<impl IntoResponse, ApiError> {
    // token-gated like every other subscription route.
    let (_user, _endpoints) = load_subscription(&st, token).await?;
    let size_mb = query.mb.unwrap_or(10.0).clamp(1.0, 50.0);
    let len = (size_mb * 1024.0 * 1024.0) as usize;
    let mut payload = vec![0u8; len];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = (i % 251) as u8; // incompressible-ish filler
    }
    let headers = [
        (header::CONTENT_TYPE, "application/octet-stream".to_string()),
        (header::CACHE_CONTROL, "no-store".to_string()),
    ];
    Ok((headers, payload))
}

/// Client links for the managed external services (MTProto / NaiveProxy) the
/// user can reach. tg:// for MTProto, naive+https:// for NaiveProxy.
async fn subscription_services(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<Json<Vec<JsonValue>>, ApiError> {
    let (_user_, _e) = load_subscription(&st, token).await?;
    let user = _user_;
    let services = repo::node_services_for_user(&st.pool, user.id).await?;
    let out = services
        .into_iter()
        .filter_map(|(s, addr)| {
            let secret = s.secret.clone().unwrap_or_default();
            let link = match s.kind.as_str() {
                "mtproto" => format!(
                    "tg://proxy?server={addr}&port={}&secret={secret}",
                    s.listen_port
                ),
                "naive" => {
                    let user_field = s
                        .config
                        .get("username")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("user");
                    let host = s
                        .config
                        .get("domain")
                        .and_then(JsonValue::as_str)
                        .filter(|d| !d.is_empty())
                        .unwrap_or(&addr);
                    format!(
                        "naive+https://{user_field}:{secret}@{host}:{}?padding=true#{}",
                        s.listen_port, s.name
                    )
                }
                _ => return None,
            };
            Some(json!({"kind": s.kind, "name": s.name, "link": link}))
        })
        .collect();
    Ok(Json(out))
}

async fn subscription_wg_list(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<Json<Vec<JsonValue>>, ApiError> {
    let (user, _) = load_subscription(&st, token).await?;
    let ifaces = repo::wg_interfaces_for_user(&st.pool, user.id).await?;
    Ok(Json(
        ifaces
            .into_iter()
            .map(|i| json!({"id": i.id, "name": i.name, "amnezia": i.amnezia}))
            .collect(),
    ))
}

/// Build one user's client config for a WG interface they can reach, returning
/// (conf text, safe filename base).
async fn build_wg_config(
    st: &AppState,
    user: &User,
    iface_id: Uuid,
) -> Result<(String, String), ApiError> {
    let iface = repo::wg_interfaces_for_user(&st.pool, user.id)
        .await?
        .into_iter()
        .find(|i| i.id == iface_id)
        .ok_or_else(|| ApiError::not_found("wireguard interface not available"))?;
    let peer = repo::ensure_wg_peer(&st.pool, &iface, user.id).await?;
    let node = repo::get_node(&st.pool, iface.node_id)
        .await?
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let host = iface
        .endpoint_host
        .clone()
        .filter(|h| !h.trim().is_empty())
        .unwrap_or(node.address);
    let endpoint = format!("{host}:{}", iface.listen_port);
    let amnezia = if iface.amnezia {
        serde_json::from_value::<crate::wg::AmneziaParams>(iface.amnezia_params.clone()).ok()
    } else {
        None
    };
    let conf = crate::wg::client_config(&crate::wg::ClientConfig {
        client_private: &peer.private_key,
        client_address: &peer.address,
        dns: &iface.dns,
        mtu: iface.mtu,
        server_public: &iface.public_key,
        endpoint: &endpoint,
        amnezia: amnezia.as_ref(),
    });
    let safe: String = iface
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    Ok((
        conf,
        if safe.is_empty() {
            "wireguard".into()
        } else {
            safe
        },
    ))
}

async fn subscription_wg_config(
    State(st): State<AppState>,
    Path((token, iface_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let (user, _) = load_subscription(&st, token).await?;
    let (conf, name) = build_wg_config(&st, &user, iface_id).await?;
    let headers = [
        (
            header::CONTENT_TYPE,
            "text/plain; charset=utf-8".to_string(),
        ),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}.conf\""),
        ),
    ];
    Ok((headers, conf))
}

async fn subscription_wg_qr(
    State(st): State<AppState>,
    Path((token, iface_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let (user, _) = load_subscription(&st, token).await?;
    let (conf, _) = build_wg_config(&st, &user, iface_id).await?;
    let svg =
        crate::subscription_page::qr_svg(&conf).map_err(|_| ApiError::internal("qr render"))?;
    let headers = [(
        header::CONTENT_TYPE,
        "image/svg+xml; charset=utf-8".to_string(),
    )];
    Ok((headers, svg))
}

/// Headers every subscription response carries: title, quota and refresh hint.
/// Happ otherwise falls back to displaying the subscription URL hostname.
fn sub_headers(user: &User, support_url: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(
        header::HeaderName::from_static("subscription-userinfo"),
        HeaderValue::from_str(&subscription::userinfo_header(user))
            .unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        header::HeaderName::from_static("profile-update-interval"),
        HeaderValue::from_static("12"),
    );
    if let Ok(value) = HeaderValue::from_str(&subscription::profile_title_header(user)) {
        headers.insert(header::HeaderName::from_static("profile-title"), value);
    }
    if let Some(value) = subscription::announce_header(user) {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(header::HeaderName::from_static("announce"), value);
        }
    }
    if let Some(value) = support_url.filter(|v| !v.trim().is_empty()) {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(header::HeaderName::from_static("support-url"), value);
        }
    }
    headers
}

async fn subscription_singbox(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<Json<JsonValue>, ApiError> {
    let (user, endpoints) = load_subscription(&st, token).await?;
    let profile = repo::routing_profile_for_user(&st.pool, user.id).await?;
    Ok(Json(subscription::singbox_client_config(
        &user,
        &endpoints,
        profile.as_ref(),
    )))
}

async fn subscription_singbox_tun(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<Json<JsonValue>, ApiError> {
    let (user, endpoints) = load_subscription(&st, token).await?;
    let profile = repo::routing_profile_for_user(&st.pool, user.id).await?;
    Ok(Json(subscription::singbox_tun_config(
        &user,
        &endpoints,
        profile.as_ref(),
    )))
}

async fn subscription_clash(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (user, endpoints) = load_subscription(&st, token).await?;
    let profile = repo::routing_profile_for_user(&st.pool, user.id).await?;
    let body = subscription::clash_config(&user, &endpoints, profile.as_ref());
    let mut response = ([(header::CONTENT_TYPE, "text/yaml; charset=utf-8")], body).into_response();
    let support = repo::all_settings(&st.pool)
        .await?
        .into_iter()
        .find(|(k, _)| k == "subscription_support_url")
        .map(|(_, v)| v);
    let headers = sub_headers(&user, support.as_deref());
    response.headers_mut().extend(headers);
    Ok(response)
}

async fn subscription_qr(
    State(st): State<AppState>,
    Path((token, inbound_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let (user, endpoints) = load_subscription(&st, token).await?;
    let link = subscription::endpoint_links(&user, &endpoints)
        .into_iter()
        .find(|link| link.inbound_id == inbound_id)
        .and_then(|link| link.uri)
        .ok_or_else(|| ApiError::not_found("subscription endpoint not found"))?;
    let svg = crate::subscription_page::qr_svg(&link)?;
    let mut response = svg.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

// --- client-tailored subscription (one URL for every app) ------------------

#[derive(Clone, Copy)]
enum SubFormat {
    V2ray,
    Happ,
    Clash,
    Singbox,
}

/// Map a client User-Agent to the format it expects. Returns None for browsers
/// and unknown agents (they get the HTML page / JSON as before).
fn format_for_ua(ua: &str) -> Option<SubFormat> {
    let ua = ua.to_ascii_lowercase();
    let has = |keys: &[&str]| keys.iter().any(|k| ua.contains(k));
    if has(&["clash", "mihomo", "stash", "flclash", "clashx"]) {
        return Some(SubFormat::Clash);
    }
    if has(&["sing-box", "singbox", "sfa", "sfi", "sft", "hiddify"]) {
        return Some(SubFormat::Singbox);
    }
    if has(&["happ"]) {
        return Some(SubFormat::Happ);
    }
    if has(&[
        "v2ray",
        "nekobox",
        "nekoray",
        "streisand",
        "shadowrocket",
        "v2raytun",
        "loon",
        "surge",
        "quantumult",
        "sagernet",
        "matsuri",
        "throne",
        "karing",
        "v2box",
    ]) {
        return Some(SubFormat::V2ray);
    }
    None
}

/// Render a user's subscription in the requested format, carrying the standard
/// `Subscription-Userinfo` / update-interval headers.
async fn tailored_response(
    st: &AppState,
    fmt: SubFormat,
    user: &User,
    endpoints: &[crate::db::models::SubscriptionEndpoint],
) -> Result<Response, ApiError> {
    let (content_type, body) = match fmt {
        SubFormat::V2ray => (
            "text/plain; charset=utf-8",
            subscription::v2ray_document(user, endpoints),
        ),
        SubFormat::Happ => (
            "text/plain; charset=utf-8",
            subscription::happ_v2ray_document(user, endpoints),
        ),
        SubFormat::Singbox => {
            let profile = repo::routing_profile_for_user(&st.pool, user.id).await?;
            let cfg = subscription::singbox_client_config(user, endpoints, profile.as_ref());
            (
                "application/json; charset=utf-8",
                serde_json::to_string_pretty(&cfg).unwrap_or_default(),
            )
        }
        SubFormat::Clash => {
            let profile = repo::routing_profile_for_user(&st.pool, user.id).await?;
            (
                "text/yaml; charset=utf-8",
                subscription::clash_config(user, endpoints, profile.as_ref()),
            )
        }
    };
    let mut response = ([(header::CONTENT_TYPE, content_type)], body).into_response();
    let headers = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&subscription::userinfo_header(user)) {
        headers.insert(header::HeaderName::from_static("subscription-userinfo"), v);
    }
    headers.insert(
        header::HeaderName::from_static("profile-update-interval"),
        HeaderValue::from_static("12"),
    );
    if let Ok(v) = HeaderValue::from_str(&subscription::profile_title_header(user)) {
        headers.insert(header::HeaderName::from_static("profile-title"), v);
    }
    if let Some(value) = subscription::announce_header(user) {
        if let Ok(v) = HeaderValue::from_str(&value) {
            headers.insert(header::HeaderName::from_static("announce"), v);
        }
    }
    if let Some(value) = repo::all_settings(&st.pool)
        .await?
        .into_iter()
        .find(|(k, _)| k == "subscription_support_url")
        .map(|(_, v)| v)
        .filter(|v| !v.trim().is_empty())
    {
        if let Ok(v) = HeaderValue::from_str(&value) {
            headers.insert(header::HeaderName::from_static("support-url"), v);
        }
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Serve a subscription by its short alias (`/s/:alias`). Client apps get their
/// tailored config; browsers are redirected to the canonical dashboard.
async fn subscription_by_alias(
    State(st): State<AppState>,
    Path(alias): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let mut user = repo::get_user_by_alias(&st.pool, &alias)
        .await?
        .ok_or_else(|| ApiError::not_found("subscription not found"))?;
    apply_subscription_defaults(&st, &mut user).await?;
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    if let Some(fmt) = ua.and_then(format_for_ua) {
        if let Some(reason) = user.suppressed_reason() {
            return Err(ApiError::gone(format!("subscription is {reason}")));
        }
        let endpoints =
            subscription::expand_endpoints(repo::subscription_endpoints(&st.pool, user.id).await?);
        return tailored_response(&st, fmt, &user, &endpoints).await;
    }
    // a browser: bounce to /sub/{token} when we can recover the token.
    if let Some(Some(token)) = repo::reveal_subscription_token(&st.pool, user.id).await? {
        return Ok(axum::response::Redirect::to(&format!("/sub/{token}")).into_response());
    }
    // legacy user without a revealable token: hand back the universal base64 sub.
    if let Some(reason) = user.suppressed_reason() {
        return Err(ApiError::gone(format!("subscription is {reason}")));
    }
    let endpoints =
        subscription::expand_endpoints(repo::subscription_endpoints(&st.pool, user.id).await?);
    tailored_response(&st, SubFormat::V2ray, &user, &endpoints).await
}

/// QR encoding the whole-subscription URL (alias if set, else token) so a single
/// scan imports every endpoint into a client.
async fn subscription_qr_all(
    State(st): State<AppState>,
    Path(token): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = repo::get_user_by_subscription_token(&st.pool, token)
        .await?
        .ok_or_else(|| ApiError::not_found("subscription not found"))?;
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("https");
    let path = match &user.subscription_alias {
        Some(alias) => format!("/s/{alias}"),
        None => format!("/sub/{token}"),
    };
    let svg = crate::subscription_page::qr_svg(&format!("{scheme}://{host}{path}"))?;
    let mut response = svg.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[derive(Deserialize)]
struct AliasInput {
    #[serde(default)]
    alias: Option<String>,
}

async fn set_alias(
    State(st): State<AppState>,
    Extension(identity): Extension<Identity>,
    Path(id): Path<Uuid>,
    Json(input): Json<AliasInput>,
) -> Result<Json<UserView>, ApiError> {
    owned_user(&st, &identity, id).await?;
    let alias = match input
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(a) => {
            if a.len() < 3
                || a.len() > 40
                || !a
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(ApiError::bad_request(
                    "alias must be 3-40 chars: letters, digits, - or _",
                ));
            }
            Some(a.to_string())
        }
        None => None,
    };
    let updated = repo::set_user_alias(&st.pool, id, alias.as_deref())
        .await
        .map_err(|error| {
            let text = error.to_string();
            if text.contains("duplicate") || text.contains("unique") {
                ApiError::conflict("that alias is already taken")
            } else {
                ApiError::internal(text)
            }
        })?;
    if !updated {
        return Err(ApiError::not_found("user not found"));
    }
    audit(&st, &identity, "set_alias", "user", Some(id), json!({})).await;
    Ok(Json(
        repo::get_user(&st.pool, id)
            .await?
            .ok_or_else(|| ApiError::not_found("user not found"))?
            .into(),
    ))
}

async fn load_subscription_page(
    st: &AppState,
    token: Uuid,
) -> Result<(User, Vec<crate::db::models::SubscriptionEndpoint>), ApiError> {
    let mut user = repo::get_user_by_subscription_token(&st.pool, token)
        .await?
        .ok_or_else(|| ApiError::not_found("subscription not found"))?;
    apply_subscription_defaults(st, &mut user).await?;
    let endpoints = if user.is_active() {
        repo::subscription_endpoints(&st.pool, user.id).await?
    } else {
        Vec::new()
    };
    Ok((user, endpoints))
}

async fn load_subscription(
    st: &AppState,
    token: Uuid,
) -> Result<(User, Vec<crate::db::models::SubscriptionEndpoint>), ApiError> {
    let mut user = repo::get_user_by_subscription_token(&st.pool, token)
        .await?
        .ok_or_else(|| {
            tracing::info!(code = "M0702", "unknown sub token, sending them away");
            ApiError::not_found("subscription not found")
        })?;
    apply_subscription_defaults(st, &mut user).await?;
    if let Some(reason) = user.suppressed_reason() {
        tracing::info!(code = "M0703", user = %user.id, reason, "sub is gone");
        return Err(ApiError::gone(format!("subscription is {reason}")));
    }
    let endpoints =
        subscription::expand_endpoints(repo::subscription_endpoints(&st.pool, user.id).await?);
    Ok((user, endpoints))
}

async fn apply_subscription_defaults(st: &AppState, user: &mut User) -> Result<(), ApiError> {
    let settings: std::collections::HashMap<String, String> =
        repo::all_settings(&st.pool).await?.into_iter().collect();
    if user
        .subscription_title
        .as_deref()
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        if let Some(value) = settings.get("default_subscription_title") {
            if !value.trim().is_empty() {
                user.subscription_title = Some(value.clone());
            }
        }
    }
    if user
        .subscription_description
        .as_deref()
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        if let Some(value) = settings.get("default_subscription_description") {
            if !value.trim().is_empty() {
                user.subscription_description = Some(value.clone());
            }
        }
    }
    Ok(())
}

async fn push_nodes(st: &AppState, ids: impl IntoIterator<Item = Uuid>) {
    if repo::setting_i64(&st.pool, "auto_push_enabled", 1).await == 0 {
        return;
    }
    for id in ids.into_iter().collect::<HashSet<_>>() {
        if st.registry.is_connected(id).await {
            // Let the mutation response and the panel's follow-up reads leave
            // the socket before a core apply can restart an Xray fallback that
            // carries this very control-plane connection.
            st.registry
                .defer_push(id, std::time::Duration::from_secs(5))
                .await;
        }
    }
}
fn validate_node(input: &NewNode) -> Result<(), ApiError> {
    if input.tls_server_name.trim().is_empty() {
        return Err(ApiError::bad_request("tls_server_name must not be empty"));
    }
    validate_name_address_port_transport(
        &input.name,
        &input.address,
        input.grpc_port,
        &input.transport,
    )
}
fn validate_update_node(input: &UpdateNode) -> Result<(), ApiError> {
    if input
        .tls_server_name
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(ApiError::bad_request("tls_server_name must not be empty"));
    }
    if input.name.as_deref().is_some_and(|v| v.trim().is_empty()) {
        return Err(ApiError::bad_request("node name must not be empty"));
    }
    if input
        .address
        .as_deref()
        .is_some_and(|v| v.trim().is_empty())
    {
        return Err(ApiError::bad_request("node address must not be empty"));
    }
    if input.grpc_port.is_some_and(|v| !(1..=65_535).contains(&v)) {
        return Err(ApiError::bad_request(
            "grpc_port must be between 1 and 65535",
        ));
    }
    if input
        .transport
        .as_deref()
        .is_some_and(|v| !matches!(v, "serve" | "dial" | "both"))
    {
        return Err(ApiError::bad_request(
            "transport must be serve, dial, or both",
        ));
    }
    Ok(())
}
fn validate_name_address_port_transport(
    name: &str,
    address: &str,
    port: i32,
    transport: &str,
) -> Result<(), ApiError> {
    if name.trim().is_empty() {
        return Err(ApiError::bad_request("node name must not be empty"));
    }
    if address.trim().is_empty() {
        return Err(ApiError::bad_request("node address must not be empty"));
    }
    if !(1..=65_535).contains(&port) {
        return Err(ApiError::bad_request(
            "grpc_port must be between 1 and 65535",
        ));
    }
    if !matches!(transport, "serve" | "dial" | "both") {
        return Err(ApiError::bad_request(
            "transport must be serve, dial, or both",
        ));
    }
    Ok(())
}
/// Chainable exit protocols (must have a sing-box outbound builder).
const CHAINABLE: &[&str] = &[
    "vless",
    "vmess",
    "trojan",
    "hysteria2",
    "tuic",
    "shadowsocks",
];

/// Validate a multihop upstream target: it must exist, run on sing-box, use a
/// chainable protocol, not be the inbound itself, and not chain straight back to
/// it (a one-level cycle).
async fn validate_upstream(
    pool: &PgPool,
    upstream_id: Option<Uuid>,
    self_id: Option<Uuid>,
    entry_core: &str,
) -> Result<(), ApiError> {
    let Some(upstream_id) = upstream_id else {
        return Ok(());
    };
    if entry_core == "xray" {
        return Err(ApiError::bad_request(
            "multihop entry must run on sing-box (the chain outbound is sing-box)",
        ));
    }
    if self_id == Some(upstream_id) {
        return Err(ApiError::bad_request("an inbound cannot chain to itself"));
    }
    let exit = repo::get_inbound(pool, upstream_id)
        .await?
        .ok_or_else(|| ApiError::bad_request("upstream inbound not found"))?;
    if exit.core != "singbox" {
        return Err(ApiError::bad_request("multihop exit must run on sing-box"));
    }
    if !CHAINABLE.contains(&exit.kind.as_str()) {
        return Err(ApiError::bad_request(format!(
            "protocol '{}' cannot be a multihop exit",
            exit.kind
        )));
    }
    // walk the chain from the exit: if it leads back to us (A→B→…→A) it forms a
    // traffic loop between nodes. Bounded depth also caps how deep chains nest.
    if let Some(self_id) = self_id {
        let mut hop = exit.upstream_inbound_id;
        for _ in 0..8 {
            let Some(next) = hop else { break };
            if next == self_id {
                return Err(ApiError::bad_request("that chain leads back here (cycle)"));
            }
            hop = match repo::get_inbound(pool, next).await? {
                Some(ib) => ib.upstream_inbound_id,
                None => break,
            };
        }
        if hop.is_some() {
            return Err(ApiError::bad_request(
                "multihop chain is too deep (max 8 hops)",
            ));
        }
    }
    Ok(())
}

fn validate_inbound(input: &NewInbound) -> Result<(), ApiError> {
    validate_inbound_basics(
        &input.tag,
        &input.kind,
        &input.core,
        input.listen_port,
        &input.extra,
    )?;
    if input.reality && !input.tls_enabled {
        return Err(ApiError::bad_request("reality requires tls_enabled=true"));
    }
    if input.reality
        && input
            .reality_private_key
            .as_deref()
            .unwrap_or("")
            .is_empty()
    {
        return Err(ApiError::bad_request("reality_private_key is required"));
    }
    if input.reality && input.reality_public_key.as_deref().unwrap_or("").is_empty() {
        return Err(ApiError::bad_request("reality_public_key is required"));
    }
    let acme = acme_enabled(&input.extra);
    if acme {
        if input.reality {
            return Err(ApiError::bad_request(
                "acme and reality are mutually exclusive",
            ));
        }
        if !input.tls_enabled {
            return Err(ApiError::bad_request("acme requires tls_enabled=true"));
        }
        if input.server_name.as_deref().unwrap_or("").trim().is_empty() {
            return Err(ApiError::bad_request(
                "acme requires a server_name (the domain to certify)",
            ));
        }
        if acme_email(&input.extra).is_none() {
            return Err(ApiError::bad_request(
                "acme requires an email (extra.acme.email)",
            ));
        }
        validate_acme_for_core(&input.core, &input.extra)?;
    }
    validate_effective_security(
        &input.kind,
        &input.core,
        input.tls_enabled,
        input.server_name.as_deref(),
        input.cert_path.as_deref(),
        input.key_path.as_deref(),
        input.reality,
        input.reality_private_key.as_deref(),
        input.reality_public_key.as_deref(),
        &input.reality_short_ids,
        input.reality_handshake_server.as_deref(),
        &input.network,
        acme,
    )?;
    if !is_valid_network(&input.network) {
        return Err(ApiError::bad_request(
            "network must be one of tcp/ws/grpc/http/h2/httpupgrade/xhttp/quic/mkcp",
        ));
    }
    validate_vless_flow(&input.kind, &input.network, input.tls_enabled, &input.flow)?;
    if input
        .shadowtls_handshake_port
        .is_some_and(|port| !(1..=65_535).contains(&port))
    {
        return Err(ApiError::bad_request(
            "shadowtls_handshake_port must be between 1 and 65535",
        ));
    }
    if input.kind == "shadowtls"
        && input
            .shadowtls_handshake_server
            .as_deref()
            .unwrap_or("")
            .is_empty()
    {
        return Err(ApiError::bad_request(
            "shadowtls requires a handshake server",
        ));
    }
    Ok(())
}

fn is_valid_network(network: &str) -> bool {
    matches!(
        network,
        "tcp" | "ws" | "grpc" | "http" | "h2" | "httpupgrade" | "xhttp" | "quic" | "mkcp"
    )
}

fn validate_vless_flow(
    kind: &str,
    network: &str,
    tls_enabled: bool,
    flow: &str,
) -> Result<(), ApiError> {
    let flow = flow.trim();
    if flow.is_empty() {
        return Ok(());
    }
    if kind != "vless" {
        return Err(ApiError::bad_request(
            "flow is only supported for vless inbounds",
        ));
    }
    if flow != "xtls-rprx-vision" {
        return Err(ApiError::bad_request(
            "unsupported VLESS flow; use xtls-rprx-vision or leave it empty",
        ));
    }
    if network != "tcp" || !tls_enabled {
        return Err(ApiError::bad_request(
            "xtls-rprx-vision requires VLESS over TCP with TLS or REALITY",
        ));
    }
    Ok(())
}

/// ACME contact: extra.acme.email, or a legacy top-level extra.acme_email.
fn acme_email(extra: &JsonValue) -> Option<&str> {
    extra
        .get("acme")
        .and_then(|a| a.get("email"))
        .or_else(|| extra.get("acme_email"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
}
fn validate_update_inbound(input: &UpdateInbound) -> Result<(), ApiError> {
    if input.tag.as_deref().is_some_and(|v| v.trim().is_empty())
        || input.kind.as_deref().is_some_and(|v| v.trim().is_empty())
    {
        return Err(ApiError::bad_request(
            "inbound tag and kind must not be empty",
        ));
    }
    if input
        .core
        .as_deref()
        .is_some_and(|v| !matches!(v, "singbox" | "xray"))
    {
        return Err(ApiError::bad_request("core must be singbox or xray"));
    }
    if input
        .listen_port
        .is_some_and(|v| !(1..=65_535).contains(&v))
    {
        return Err(ApiError::bad_request(
            "listen_port must be between 1 and 65535",
        ));
    }
    if input.extra.as_ref().is_some_and(|v| !v.is_object()) {
        return Err(ApiError::bad_request("extra must be a JSON object"));
    }
    if matches!(input.reality_handshake_port, Patch::Value(v) if !(1..=65_535).contains(&v)) {
        return Err(ApiError::bad_request(
            "reality_handshake_port must be between 1 and 65535",
        ));
    }
    if input
        .network
        .as_deref()
        .is_some_and(|n| !is_valid_network(n))
    {
        return Err(ApiError::bad_request("unsupported network transport"));
    }
    if input
        .flow
        .as_deref()
        .is_some_and(|flow| !flow.trim().is_empty() && flow != "xtls-rprx-vision")
    {
        return Err(ApiError::bad_request(
            "unsupported VLESS flow; use xtls-rprx-vision or leave it empty",
        ));
    }
    if matches!(
        input.shadowtls_handshake_port,
        Patch::Value(port) if !(1..=65_535).contains(&port)
    ) {
        return Err(ApiError::bad_request(
            "shadowtls_handshake_port must be between 1 and 65535",
        ));
    }
    Ok(())
}
fn validate_effective_update(current: &Inbound, input: &UpdateInbound) -> Result<(), ApiError> {
    let kind = input.kind.as_deref().unwrap_or(&current.kind);
    let core = input.core.as_deref().unwrap_or(&current.core);
    let tls_enabled = input.tls_enabled.unwrap_or(current.tls_enabled);
    let reality = input.reality.unwrap_or(current.reality);
    let network = input.network.as_deref().unwrap_or(&current.network);
    let flow = input.flow.as_deref().unwrap_or(&current.flow);
    let short_ids = input
        .reality_short_ids
        .as_deref()
        .unwrap_or(&current.reality_short_ids);
    let extra = input.extra.as_ref().unwrap_or(&current.extra);
    let acme = acme_enabled(extra);
    validate_vless_flow(kind, network, tls_enabled, flow)?;
    validate_effective_security(
        kind,
        core,
        tls_enabled,
        effective_string(&input.server_name, &current.server_name),
        effective_string(&input.cert_path, &current.cert_path),
        effective_string(&input.key_path, &current.key_path),
        reality,
        effective_string(&input.reality_private_key, &current.reality_private_key),
        effective_string(&input.reality_public_key, &current.reality_public_key),
        short_ids,
        effective_string(
            &input.reality_handshake_server,
            &current.reality_handshake_server,
        ),
        network,
        acme,
    )?;
    let extra = input.extra.as_ref().unwrap_or(&current.extra);
    validate_acme_for_core(core, extra)
}

fn effective_string<'a>(patch: &'a Patch<String>, current: &'a Option<String>) -> Option<&'a str> {
    match patch {
        Patch::Missing => current.as_deref(),
        Patch::Null => None,
        Patch::Value(value) => Some(value),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_effective_security(
    kind: &str,
    core: &str,
    tls_enabled: bool,
    server_name: Option<&str>,
    cert_path: Option<&str>,
    key_path: Option<&str>,
    reality: bool,
    reality_private_key: Option<&str>,
    reality_public_key: Option<&str>,
    reality_short_ids: &[String],
    reality_handshake_server: Option<&str>,
    network: &str,
    acme: bool,
) -> Result<(), ApiError> {
    let supported = match core {
        "singbox" => matches!(
            kind,
            "vless"
                | "hysteria2"
                | "vmess"
                | "trojan"
                | "shadowsocks"
                | "tuic"
                | "anytls"
                | "shadowtls"
        ),
        "xray" => matches!(kind, "vless" | "vmess" | "trojan" | "shadowsocks"),
        _ => false,
    };
    if !supported {
        return Err(ApiError::bad_request(format!(
            "protocol '{kind}' is not supported by core '{core}'"
        )));
    }

    let network_supported = match core {
        "singbox" => matches!(
            network,
            "tcp" | "ws" | "grpc" | "http" | "h2" | "httpupgrade" | "quic"
        ),
        "xray" => matches!(
            network,
            "tcp" | "ws" | "grpc" | "http" | "h2" | "httpupgrade" | "xhttp" | "quic" | "mkcp"
        ),
        _ => false,
    };
    if !network_supported {
        return Err(ApiError::bad_request(format!(
            "transport '{network}' is not supported by core '{core}'"
        )));
    }
    if kind == "hysteria2" {
        if !tls_enabled {
            return Err(ApiError::bad_request("hysteria2 requires tls_enabled=true"));
        }
        if reality {
            return Err(ApiError::bad_request("hysteria2 does not support reality"));
        }
    }

    if tls_enabled && !reality && !acme {
        if cert_path.is_none_or(str::is_empty) || key_path.is_none_or(str::is_empty) {
            return Err(ApiError::bad_request(
                "tls requires both cert_path and key_path (or enable acme)",
            ));
        }
    }

    if reality {
        if kind != "vless" {
            return Err(ApiError::bad_request(
                "reality is supported only with vless",
            ));
        }
        if server_name.is_none_or(str::is_empty) {
            return Err(ApiError::bad_request("reality requires server_name"));
        }
        if reality_handshake_server.is_none_or(str::is_empty) {
            return Err(ApiError::bad_request(
                "reality requires reality_handshake_server",
            ));
        }
        if !reality_private_key.is_some_and(valid_reality_key) {
            return Err(ApiError::bad_request(
                "reality_private_key must be an unpadded base64url X25519 key",
            ));
        }
        if !reality_public_key.is_some_and(valid_reality_key) {
            return Err(ApiError::bad_request(
                "reality_public_key must be an unpadded base64url X25519 key",
            ));
        }
        if reality_short_ids.is_empty() || reality_short_ids.iter().any(|id| !valid_short_id(id)) {
            return Err(ApiError::bad_request(
                "reality_short_ids must contain even-length hexadecimal IDs (2-16 chars)",
            ));
        }
        if core == "xray" && !matches!(network, "tcp" | "xhttp" | "grpc") {
            return Err(ApiError::bad_request(
                "xray reality supports only tcp, xhttp, or grpc transport",
            ));
        }
    }
    Ok(())
}

fn validate_acme_for_core(core: &str, extra: &JsonValue) -> Result<(), ApiError> {
    let Some(acme) = extra
        .get("acme")
        .filter(|value| !value.is_null() && !value.is_boolean())
    else {
        return Ok(());
    };
    if acme_email(extra).is_none() {
        return Err(ApiError::bad_request(
            "acme requires an email (extra.acme.email)",
        ));
    }
    if core == "singbox" {
        return Ok(());
    }
    if core != "xray" {
        return Err(ApiError::bad_request(
            "acme is supported only by sing-box or xray",
        ));
    }
    let Some(acme) = acme.as_object() else {
        return Err(ApiError::bad_request(
            "xray acme settings must be an object",
        ));
    };
    if acme
        .get("disable_http_challenge")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Err(ApiError::bad_request("xray acme supports HTTP-01 only"));
    }
    if acme.get("dns01_challenge").is_some() {
        return Err(ApiError::bad_request("xray acme does not support DNS-01"));
    }
    if let Some(port) = acme.get("alternative_http_port") {
        if port.as_u64() != Some(9080) {
            return Err(ApiError::bad_request(
                "xray acme alternative_http_port must be 9080",
            ));
        }
    }
    Ok(())
}

fn acme_enabled(extra: &JsonValue) -> bool {
    extra
        .get("acme")
        .is_some_and(|value| !value.is_null() && value != &JsonValue::Bool(false))
}

fn valid_reality_key(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_short_id(value: &str) -> bool {
    (2..=16).contains(&value.len())
        && value.len() % 2 == 0
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_inbound_basics(
    tag: &str,
    kind: &str,
    core: &str,
    port: i32,
    extra: &JsonValue,
) -> Result<(), ApiError> {
    if tag.trim().is_empty() || kind.trim().is_empty() {
        return Err(ApiError::bad_request(
            "inbound tag and kind must not be empty",
        ));
    }
    if !matches!(core, "singbox" | "xray") {
        return Err(ApiError::bad_request("core must be singbox or xray"));
    }
    if !(1..=65_535).contains(&port) {
        return Err(ApiError::bad_request(
            "listen_port must be between 1 and 65535",
        ));
    }
    if !extra.is_object() {
        return Err(ApiError::bad_request("extra must be a JSON object"));
    }
    if let Some(happ) = extra.get("happ") {
        let happ = happ
            .as_object()
            .ok_or_else(|| ApiError::bad_request("extra.happ must be an object"))?;
        for (key, max) in [("name", 80usize), ("description", 30usize)] {
            if let Some(value) = happ.get(key) {
                let value = value.as_str().ok_or_else(|| {
                    ApiError::bad_request(format!("extra.happ.{key} must be text"))
                })?;
                if value.trim().chars().count() > max {
                    return Err(ApiError::bad_request(format!(
                        "extra.happ.{key} must not exceed {max} characters"
                    )));
                }
            }
        }
        if let Some(code) = happ.get("country_code") {
            let code = code
                .as_str()
                .ok_or_else(|| ApiError::bad_request("extra.happ.country_code must be text"))?;
            if !code.is_empty()
                && (code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_alphabetic()))
            {
                return Err(ApiError::bad_request(
                    "extra.happ.country_code must be a two-letter ISO code",
                ));
            }
        }
    }
    if let Some(acme) = extra.get("acme").and_then(JsonValue::as_object) {
        if let Some(port) = acme.get("alternative_http_port") {
            let port = port.as_u64().ok_or_else(|| {
                ApiError::bad_request("extra.acme.alternative_http_port must be a port")
            })?;
            if !(1..=65_535).contains(&port) {
                return Err(ApiError::bad_request(
                    "extra.acme.alternative_http_port must be between 1 and 65535",
                ));
            }
        }
        let http_disabled = acme
            .get("disable_http_challenge")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let tls_disabled = acme
            .get("disable_tls_alpn_challenge")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        if http_disabled && tls_disabled && acme.get("dns01_challenge").is_none() {
            return Err(ApiError::bad_request(
                "ACME cannot disable both HTTP-01 and TLS-ALPN-01 without DNS-01",
            ));
        }
    }
    Ok(())
}
fn validate_user(input: &NewUser) -> Result<(), ApiError> {
    if input.username.trim().is_empty() || input.password.is_empty() {
        return Err(ApiError::bad_request(
            "username and password must not be empty",
        ));
    }
    if input.traffic_limit_bytes < 0 {
        return Err(ApiError::bad_request(
            "traffic_limit_bytes must not be negative",
        ));
    }
    validate_subscription_title(input.subscription_title.as_deref())?;
    validate_subscription_description(input.subscription_description.as_deref())?;
    Ok(())
}
fn validate_update_user(input: &UpdateUser) -> Result<(), ApiError> {
    if input
        .username
        .as_deref()
        .is_some_and(|v| v.trim().is_empty())
        || input.password.as_deref().is_some_and(str::is_empty)
    {
        return Err(ApiError::bad_request(
            "username and password must not be empty",
        ));
    }
    if input.traffic_limit_bytes.is_some_and(|v| v < 0) {
        return Err(ApiError::bad_request(
            "traffic_limit_bytes must not be negative",
        ));
    }
    if let Patch::Value(title) = &input.subscription_title {
        validate_subscription_title(Some(title))?;
    }
    if let Patch::Value(description) = &input.subscription_description {
        validate_subscription_description(Some(description))?;
    }
    Ok(())
}

fn validate_subscription_title(title: Option<&str>) -> Result<(), ApiError> {
    if let Some(title) = title {
        let length = title.trim().chars().count();
        if !(1..=25).contains(&length) {
            return Err(ApiError::bad_request(
                "subscription_title must contain 1 to 25 characters",
            ));
        }
    }
    Ok(())
}

fn validate_subscription_description(description: Option<&str>) -> Result<(), ApiError> {
    if let Some(value) = description {
        let length = value.trim().chars().count();
        if !(1..=200).contains(&length) {
            return Err(ApiError::bad_request(
                "subscription_description must contain 1 to 200 characters",
            ));
        }
    }
    Ok(())
}

pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}
impl ApiError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "M1207",
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        let detail = message.into();
        tracing::error!(code = "M1202", error = %detail, "internal api error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "M1202",
            message: "internal server error".into(),
        }
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "M1201",
            message: message.into(),
        }
    }
    fn bad_gateway(message: impl Into<String>) -> Self {
        let detail = message.into();
        tracing::warn!(code = "M1203", error = %detail, "upstream request failed");
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "M1203",
            message: "upstream service request failed".into(),
        }
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "M1204",
            message: message.into(),
        }
    }
    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "M1211",
            message: message.into(),
        }
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "M1205",
            message: message.into(),
        }
    }
    fn gone(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::GONE,
            code: "M1206",
            message: message.into(),
        }
    }
    fn too_many(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "M1208",
            message: message.into(),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error": self.message, "code": self.code})),
        )
            .into_response()
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        if let Some(sqlx::Error::Database(db)) = error.downcast_ref::<sqlx::Error>() {
            return match db.code().as_deref() {
                Some("23505") => Self::conflict("resource already exists"),
                Some("23503") => {
                    Self::bad_request("referenced resource does not exist or is still in use")
                }
                Some("23514") => Self::bad_request("value violates a database constraint"),
                _ => {
                    tracing::error!(code = "M1202", %error, "database request failed");
                    Self {
                        status: StatusCode::INTERNAL_SERVER_ERROR,
                        code: "M1202",
                        message: "internal server error".into(),
                    }
                }
            };
        }
        tracing::error!(code = "M1202", %error, "api request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "M1202",
            message: "internal server error".into(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_lifecycle_inputs() {
        let node = NewNode {
            name: "node".into(),
            address: "127.0.0.1".into(),
            tls_server_name: "honey-agent".into(),
            grpc_port: 0,
            transport: "serve".into(),
            monthly_cost_cents: 0,
        };
        assert!(validate_node(&node).is_err());
        let user = NewUser {
            username: "alice".into(),
            password: "secret".into(),
            subscription_title: None,
            subscription_description: None,
            traffic_limit_bytes: -1,
            expires_at: None,
            device_limit: 0,
        };
        assert!(validate_user(&user).is_err());
    }

    fn valid_reality_inbound() -> NewInbound {
        NewInbound {
            node_id: Uuid::new_v4(),
            tag: "vless-in".into(),
            kind: "vless".into(),
            core: "xray".into(),
            listen_port: 443,
            flow: "xtls-rprx-vision".into(),
            tls_enabled: true,
            server_name: Some("www.cloudflare.com".into()),
            cert_path: None,
            key_path: None,
            reality: true,
            reality_private_key: Some("UuMBgl7MXTPx9inmQp2UC7Jcnwc6XYbwDNebonM-FCc".into()),
            reality_public_key: Some("jNXHt1yRo0vDuchQlIP6Z0ZvjT3KtzVI-T4E7RoLJS0".into()),
            reality_short_ids: vec!["0123456789abcdef".into()],
            reality_handshake_server: Some("www.cloudflare.com".into()),
            reality_handshake_port: Some(443),
            network: "tcp".into(),
            transport_path: None,
            transport_host: None,
            transport_service_name: None,
            transport_mode: None,
            ech: false,
            utls_fingerprint: Some("chrome".into()),
            shadowtls_handshake_server: None,
            shadowtls_handshake_port: None,
            extra: json!({}),
            fallback_host: None,
            sni_pool: vec![],
            cdn_pool: vec![],
            up_mbps: 0,
            down_mbps: 0,
            upstream_inbound_id: None,
        }
    }

    fn stored_inbound() -> Inbound {
        let now = Utc::now();
        Inbound {
            id: Uuid::new_v4(),
            node_id: Uuid::new_v4(),
            tag: "test-in".into(),
            labels: vec![],
            kind: "vless".into(),
            core: "xray".into(),
            listen: "::".into(),
            listen_port: 443,
            flow: "xtls-rprx-vision".into(),
            tls_enabled: true,
            server_name: Some("example.com".into()),
            cert_path: None,
            key_path: None,
            reality: true,
            reality_private_key: Some("private-material-must-never-leave-the-api".into()),
            reality_public_key: Some("public-material".into()),
            reality_short_ids: vec!["0123456789abcdef".into()],
            reality_handshake_server: Some("example.com".into()),
            reality_handshake_port: Some(443),
            network: "tcp".into(),
            transport_path: None,
            transport_host: None,
            transport_service_name: None,
            transport_mode: None,
            ech: false,
            utls_fingerprint: Some("chrome".into()),
            shadowtls_handshake_server: None,
            shadowtls_handshake_port: None,
            extra: json!({}),
            enabled: true,
            reachable: None,
            reach_checked_at: None,
            reach_error: None,
            fallback_host: None,
            sni_pool: vec![],
            cdn_pool: vec![],
            up_mbps: 0,
            down_mbps: 0,
            upstream_inbound_id: None,
            chain_uuid: None,
            chain_password: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn accepts_documented_reality_vless() {
        assert!(validate_inbound(&valid_reality_inbound()).is_ok());
    }

    #[test]
    fn rejects_invalid_reality_material() {
        let mut inbound = valid_reality_inbound();
        inbound.reality_short_ids = vec!["odd".into()];
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.reality_private_key = Some("not-a-key".into());
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.reality_handshake_server = None;
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.kind = "trojan".into();
        assert!(validate_inbound(&inbound).is_err());
    }

    #[test]
    fn enforces_protocol_core_and_tls_compatibility() {
        let mut inbound = valid_reality_inbound();
        inbound.kind = "anytls".into();
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.kind = "hysteria2".into();
        inbound.core = "singbox".into();
        inbound.flow.clear();
        inbound.reality = false;
        inbound.cert_path = Some("/etc/honey/fullchain.pem".into());
        inbound.key_path = Some("/etc/honey/privkey.pem".into());
        assert!(validate_inbound(&inbound).is_ok());
    }

    #[test]
    fn enforces_core_transport_and_acme_compatibility() {
        let mut inbound = valid_reality_inbound();
        inbound.core = "singbox".into();
        inbound.network = "xhttp".into();
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.reality = false;
        inbound.cert_path = None;
        inbound.key_path = None;
        inbound.extra = json!({"acme": {"email": "ops@example.com"}});
        assert!(validate_inbound(&inbound).is_ok());

        inbound.extra = json!({"acme": {
            "email": "ops@example.com",
            "disable_http_challenge": true
        }});
        assert!(validate_inbound(&inbound).is_err());

        inbound.extra = json!({"acme": {
            "email": "ops@example.com",
            "alternative_http_port": 9082
        }});
        assert!(validate_inbound(&inbound).is_err());

        inbound.extra = json!({});
        inbound.cert_path = Some("/etc/honey/fullchain.pem".into());
        inbound.key_path = Some("/etc/honey/privkey.pem".into());
        inbound.network = "xhttp".into();
        inbound.flow.clear();
        assert!(validate_inbound(&inbound).is_ok());
    }

    #[test]
    fn enforces_vless_flow_compatibility() {
        let mut inbound = valid_reality_inbound();
        inbound.flow = "unsupported-flow".into();
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.network = "xhttp".into();
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.kind = "hysteria2".into();
        inbound.flow = "xtls-rprx-vision".into();
        assert!(validate_inbound(&inbound).is_err());

        let mut inbound = valid_reality_inbound();
        inbound.flow.clear();
        inbound.network = "xhttp".into();
        assert!(validate_inbound(&inbound).is_ok());
    }

    #[test]
    fn patch_distinguishes_missing_and_null() {
        let input: UpdateUser = serde_json::from_str(r#"{"expires_at":null}"#).unwrap();
        assert!(matches!(input.expires_at, Patch::Null));
        let input: UpdateUser = serde_json::from_str("{}").unwrap();
        assert!(matches!(input.expires_at, Patch::Missing));
    }

    #[test]
    fn inbound_view_hides_private_key_and_reports_certificate_state() {
        let reality = serde_json::to_value(InboundView::from(stored_inbound())).unwrap();
        assert!(reality.get("reality_private_key").is_none());
        assert_eq!(reality["certificate_source"], "reality");
        assert_eq!(reality["certificate_status"], "not_applicable");

        let mut acme = stored_inbound();
        acme.reality = false;
        acme.core = "singbox".into();
        acme.extra = json!({"acme": {"email": "ops@example.com"}});
        let acme = serde_json::to_value(InboundView::from(acme)).unwrap();
        assert_eq!(acme["certificate_source"], "acme");
        assert_eq!(acme["certificate_status"], "managed");

        let mut xray_acme = stored_inbound();
        xray_acme.reality = false;
        xray_acme.core = "xray".into();
        xray_acme.extra = json!({"acme": {"email": "ops@example.com"}});
        let xray_acme = serde_json::to_value(InboundView::from(xray_acme)).unwrap();
        assert_eq!(xray_acme["certificate_source"], "acme");
        assert_eq!(xray_acme["certificate_status"], "managed");

        let mut manual = stored_inbound();
        manual.reality = false;
        manual.cert_path = Some("/etc/honey/fullchain.pem".into());
        manual.key_path = Some("/etc/honey/privkey.pem".into());
        let configured = serde_json::to_value(InboundView::from(manual.clone())).unwrap();
        assert_eq!(configured["certificate_status"], "configured");
        manual.key_path = None;
        let missing = serde_json::to_value(InboundView::from(manual)).unwrap();
        assert_eq!(missing["certificate_status"], "missing");
    }

    #[test]
    fn internal_and_upstream_errors_do_not_echo_details() {
        let internal = ApiError::internal("postgres://admin:secret@database/honey");
        assert_eq!(internal.message, "internal server error");
        assert!(!internal.message.contains("secret"));

        let upstream = ApiError::bad_gateway("agent rejected private_key=secret");
        assert_eq!(upstream.message, "upstream service request failed");
        assert!(!upstream.message.contains("private_key"));
    }

    #[test]
    fn labels_are_canonical_and_bounded() {
        let labels = normalize_labels(vec![
            " Region:PL ".into(),
            "production".into(),
            "region:pl".into(),
        ])
        .ok()
        .expect("valid labels");
        assert_eq!(labels, vec!["production", "region:pl"]);
        assert!(normalize_labels(vec!["contains space".into()]).is_err());
        assert!(normalize_labels((0..17).map(|i| format!("label-{i}")).collect()).is_err());
    }

    #[test]
    fn saved_view_definitions_reject_unknown_or_cross_resource_filters() {
        let mut valid = json!({
            "search": "  poland  ",
            "labels": [" Region:PL ", "region:pl"],
            "sort": "name",
            "columns": ["name", "labels", "status"]
        });
        assert!(normalize_view_definition("nodes", &mut valid).is_ok());
        assert_eq!(valid["search"], "poland");
        assert_eq!(valid["labels"], json!(["region:pl"]));

        let mut unknown = json!({"sort": "name", "private": true});
        assert!(normalize_view_definition("nodes", &mut unknown).is_err());
        let mut issue_filter = json!({"severity": "critical"});
        assert!(normalize_view_definition("users", &mut issue_filter).is_err());
    }

    #[test]
    fn label_and_saved_view_permissions_are_scoped() {
        assert_eq!(required_role(&Method::POST, "/saved-views"), "viewer");
        assert_eq!(required_role(&Method::PUT, "/users/id/labels"), "operator");
        assert!(reseller_permits(&Method::PUT, "/users/id/labels"));
        assert!(reseller_permits(&Method::POST, "/saved-views"));
        assert!(!reseller_permits(&Method::PUT, "/nodes/id/labels"));
    }

    #[test]
    fn session_routes_and_account_scope_are_enforced() {
        let own = Uuid::new_v4();
        let other = Uuid::new_v4();
        let viewer = Identity {
            admin_id: Some(own),
            username: "viewer".into(),
            role: "viewer".into(),
            session_hash: Some(vec![1]),
            permissions: None,
        };
        let admin = Identity {
            role: "admin".into(),
            ..viewer.clone()
        };
        let reseller = Identity {
            role: "reseller".into(),
            ..viewer.clone()
        };
        assert_eq!(
            required_role(&Method::DELETE, "/auth/sessions/id"),
            "viewer"
        );
        assert!(may_manage_account(&viewer, own, own));
        assert!(!may_manage_account(&viewer, own, other));
        assert!(may_manage_account(&admin, own, other));
        assert!(!may_manage_account(&reseller, own, other));
        assert!(reseller_permits(&Method::GET, "/auth/sessions"));
        assert!(reseller_permits(&Method::DELETE, "/auth/sessions/id"));
        assert!(session_account(&Identity::legacy()).is_err());
    }

    #[test]
    fn api_key_scope_lifetime_and_status_are_bounded() {
        let now = Utc::now();
        let owner = Identity {
            admin_id: Some(Uuid::new_v4()),
            username: "owner".into(),
            role: "owner".into(),
            session_hash: Some(vec![1]),
            permissions: None,
        };
        let operator = Identity {
            role: "operator".into(),
            ..owner.clone()
        };
        let reseller = Identity {
            role: "reseller".into(),
            ..owner.clone()
        };
        let input = NewApiKeyInput {
            name: " deploy bot ".into(),
            role: "operator".into(),
            expires_days: 30,
        };
        let Ok((name, role, expires_at)) = validate_api_key_input(&owner, &input, now) else {
            panic!("owner should be allowed to mint an operator key");
        };
        assert_eq!(name, "deploy bot");
        assert_eq!(role, "operator");
        assert_eq!(expires_at, Some(now + chrono::Duration::days(30)));
        assert!(validate_api_key_input(&operator, &input, now).is_ok());
        assert!(validate_api_key_input(
            &operator,
            &NewApiKeyInput {
                role: "admin".into(),
                ..input
            },
            now,
        )
        .is_err());
        assert!(validate_api_key_input(
            &reseller,
            &NewApiKeyInput {
                name: "reseller key".into(),
                role: "viewer".into(),
                expires_days: 0,
            },
            now,
        )
        .is_err());
        assert!(validate_api_key_input(
            &owner,
            &NewApiKeyInput {
                name: "bad\nname".into(),
                role: "viewer".into(),
                expires_days: 3651,
            },
            now,
        )
        .is_err());

        let key = |expires_at, revoked_at| ApiKey {
            id: Uuid::new_v4(),
            name: "ci".into(),
            role: "viewer".into(),
            created_by: owner.admin_id,
            last_used_at: None,
            expires_at,
            revoked_at,
            created_at: now,
        };
        assert_eq!(api_key_status(&key(None, None), now), "active");
        assert_eq!(
            api_key_status(&key(Some(now - chrono::Duration::seconds(1)), None), now),
            "expired"
        );
        assert_eq!(
            api_key_status(&key(None, Some(now - chrono::Duration::seconds(1))), now),
            "revoked"
        );
        let public = serde_json::to_value(api_key_view(key(None, None), now)).unwrap();
        assert!(public.get("key_hash").is_none());
        assert!(public.get("token").is_none());
        assert_eq!(required_role(&Method::GET, "/api-keys"), "owner");
        assert_eq!(required_role(&Method::POST, "/api-keys"), "owner");
        assert_eq!(required_role(&Method::DELETE, "/api-keys/id"), "owner");
        assert!(!reseller_permits(&Method::GET, "/api-keys"));
    }

    #[test]
    fn login_history_fields_are_safely_bounded() {
        assert_eq!(bounded_auth_text("   ", 96, "<empty>"), "<empty>");
        assert_eq!(bounded_auth_text("abcdef", 3, "x"), "abc");
        assert_eq!(bounded_auth_text("абвг", 3, "x"), "абв");
    }

    #[test]
    fn notification_routes_are_viewer_scoped_but_not_reseller_visible() {
        assert_eq!(required_role(&Method::GET, "/notifications"), "viewer");
        assert_eq!(
            required_role(&Method::POST, "/notifications/read-all"),
            "viewer"
        );
        assert!(!reseller_permits(&Method::GET, "/notifications"));
        assert!(validate_notification_query(&NotificationQuery::default()).is_ok());
        assert!(validate_notification_query(&NotificationQuery {
            severity: Some("critical".into()),
            event: Some("node_down".into()),
            ..Default::default()
        })
        .is_ok());
        assert!(validate_notification_query(&NotificationQuery {
            event: Some("subscription_abuse".into()),
            ..Default::default()
        })
        .is_ok());
        assert!(validate_notification_query(&NotificationQuery {
            severity: Some("debug".into()),
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn subscription_guard_parses_paths_clamps_settings_and_sets_private_headers() {
        assert_eq!(
            subscription_path_identity("/sub/550e8400-e29b-41d4-a716-446655440000/sing-box"),
            Some(("token", "550e8400-e29b-41d4-a716-446655440000"))
        );
        assert_eq!(
            subscription_path_identity("/s/friendly-alias"),
            Some(("alias", "friendly-alias"))
        );
        assert_eq!(subscription_path_identity("/health"), None);

        let settings = std::collections::HashMap::from([
            ("subscription_guard_enabled".into(), "false".into()),
            ("subscription_guard_max_requests".into(), "1".into()),
            ("subscription_guard_window_secs".into(), "99999".into()),
            ("subscription_guard_block_secs".into(), "bad".into()),
        ]);
        let config = subscription_guard_config_from_map(&settings);
        assert!(!config.enabled);
        assert_eq!(config.max_requests, 10);
        assert_eq!(config.window.as_secs(), 3600);
        assert_eq!(
            config.block.as_secs(),
            DEFAULT_SUBSCRIPTION_GUARD_BLOCK_SECS as u64
        );

        let mut response = StatusCode::OK.into_response();
        apply_subscription_security_headers(&mut response);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store, max-age=0"
        );
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
    }

    #[test]
    fn log_query_restrictions_are_bounded_and_allow_request_id_search() {
        assert!(validate_log_query(&LogQuery {
            limit: Some(100),
            level: Some("warn".into()),
            code: Some("M0406".into()),
            query: Some("request_id=abc".into()),
        })
        .is_ok());
        assert!(validate_log_query(&LogQuery {
            limit: None,
            level: Some("fatal".into()),
            code: None,
            query: None,
        })
        .is_err());
        assert!(validate_log_query(&LogQuery {
            limit: None,
            level: None,
            code: Some("M04".into()),
            query: None,
        })
        .is_err());
        assert!(validate_log_query(&LogQuery {
            limit: None,
            level: None,
            code: Some("N0406".into()),
            query: None,
        })
        .is_err());
        assert!(validate_log_query(&LogQuery {
            limit: None,
            level: None,
            code: None,
            query: Some("x".repeat(129)),
        })
        .is_err());
    }

    #[test]
    fn onboarding_is_derived_and_reseller_safe() {
        let snapshot = OnboardingSnapshot {
            domain_count: 1,
            node_count: 1,
            inbound_count: 0,
            user_count: 1,
            subscription_count: 1,
        };
        let operator = build_onboarding(snapshot.clone(), false);
        assert_eq!(operator.total, 5);
        assert_eq!(operator.completed, 4);
        assert_eq!(operator.steps[0].key, "domain");
        assert_eq!(operator.steps[2].key, "inbound");
        assert!(!operator.steps[2].complete);

        let reseller = build_onboarding(snapshot, true);
        assert_eq!(reseller.total, 2);
        assert_eq!(reseller.completed, 2);
        assert_eq!(
            reseller
                .steps
                .iter()
                .map(|step| step.key)
                .collect::<Vec<_>>(),
            vec!["user", "subscription"]
        );
        assert!(reseller_permits(&Method::GET, "/onboarding"));
    }

    #[test]
    fn traffic_ranges_are_bounded_and_choose_a_safe_bucket() {
        let now = Utc::now();
        let default = validate_traffic_query(&TrafficAnalyticsQuery::default(), now)
            .ok()
            .unwrap();
        assert_eq!(default.bucket, "hour");
        assert_eq!(default.to - default.from, Duration::hours(24));
        assert!(validate_traffic_query(
            &TrafficAnalyticsQuery {
                from: Some(now - Duration::days(32)),
                to: Some(now),
                bucket: Some("hour".into()),
                ..Default::default()
            },
            now,
        )
        .is_err());
        let long = validate_traffic_query(
            &TrafficAnalyticsQuery {
                from: Some(now - Duration::days(30)),
                to: Some(now),
                ..Default::default()
            },
            now,
        )
        .ok()
        .unwrap();
        assert_eq!(long.bucket, "day");
        assert!(validate_traffic_query(
            &TrafficAnalyticsQuery {
                core: Some("unknown".into()),
                ..Default::default()
            },
            now,
        )
        .is_err());
    }

    #[test]
    fn traffic_change_and_reseller_scope_are_explicit() {
        assert_eq!(traffic_change_percent(150, 100), Some(50.0));
        assert_eq!(traffic_change_percent(1, 0), None);
        assert!(reseller_permits(&Method::GET, "/analytics/traffic"));
        assert!(reseller_permits(&Method::GET, "/analytics/traffic.csv"));
        assert_eq!(required_role(&Method::GET, "/analytics/traffic"), "viewer");
    }
}
