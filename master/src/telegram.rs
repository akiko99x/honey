//! Interactive Telegram bot: long-polls for commands. Admin chats (allowlisted
//! with role=admin) get read ops (/status /nodes /find) plus user mutations
//! (/adduser /setquota /setexpiry /enable /disable) — every mutation is audited
//! (actor `telegram:<chat_id>`) and propagates to nodes on the next reconcile.
//! Anyone can self-serve their own subscription with a token. Configured via
//! HONEY_TELEGRAM_TOKEN; the public base URL for links via HONEY_PUBLIC_URL.

use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth;
use crate::db::models::{NewUser, Patch, UpdateUser};
use crate::db::repo;

#[derive(Deserialize)]
struct TgResponse<T> {
    #[serde(default)]
    result: Option<T>,
}

#[derive(Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    text: Option<String>,
    chat: Chat,
}

#[derive(Deserialize)]
struct Chat {
    id: i64,
}

pub async fn run(pool: PgPool, token: String, public_url: Option<String>) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(70))
        .build()?;
    let api = format!("https://api.telegram.org/bot{token}");
    tracing::info!(code = "M1600", "telegram bot up");

    let mut offset: i64 = 0;
    loop {
        // HA: only the lease holder long-polls, or several instances would each
        // consume updates and answer the same command.
        if !crate::ha::is_leader() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        let updates = match get_updates(&client, &api, offset).await {
            Ok(updates) => updates,
            Err(e) => {
                tracing::warn!(code = "M1603", "telegram getUpdates failed: {e:#}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        for update in updates {
            offset = update.update_id + 1;
            let Some(message) = update.message else {
                continue;
            };
            let Some(text) = message.text else { continue };
            let chat_id = message.chat.id;
            let reply = handle(&pool, chat_id, text.trim(), public_url.as_deref()).await;
            if let Err(e) = send_message(&client, &api, chat_id, &reply).await {
                tracing::debug!(code = "M1603", "telegram sendMessage failed: {e:#}");
            }
        }
    }
}

async fn get_updates(client: &reqwest::Client, api: &str, offset: i64) -> Result<Vec<Update>> {
    let resp: TgResponse<Vec<Update>> = client
        .get(format!("{api}/getUpdates"))
        .query(&[("timeout", "60"), ("offset", &offset.to_string())])
        .send()
        .await?
        .json()
        .await?;
    Ok(resp.result.unwrap_or_default())
}

async fn send_message(client: &reqwest::Client, api: &str, chat_id: i64, text: &str) -> Result<()> {
    client
        .post(format!("{api}/sendMessage"))
        .json(&serde_json::json!({ "chat_id": chat_id, "text": text }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn handle(pool: &PgPool, chat_id: i64, text: &str, public_url: Option<&str>) -> String {
    let mut parts = text.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    let args: Vec<&str> = parts.collect();
    let arg = args.first().copied().unwrap_or("");
    let is_admin = repo::telegram_chat_role(pool, chat_id)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("admin");

    match cmd {
        "/start" | "/help" => help_text(is_admin, chat_id),
        "/sub" => sub_reply(pool, arg, public_url).await,
        "/status" if is_admin => status_reply(pool).await,
        "/nodes" if is_admin => nodes_reply(pool).await,
        "/find" if is_admin => find_reply(pool, arg).await,
        "/adduser" if is_admin => add_user_reply(pool, &args, public_url, chat_id).await,
        "/setquota" if is_admin => set_quota_reply(pool, &args, chat_id).await,
        "/setexpiry" if is_admin => set_expiry_reply(pool, &args, chat_id).await,
        "/enable" if is_admin => toggle_reply(pool, arg, true, chat_id).await,
        "/disable" if is_admin => toggle_reply(pool, arg, false, chat_id).await,
        "/status" | "/nodes" | "/find" | "/adduser" | "/setquota" | "/setexpiry" | "/enable"
        | "/disable" => {
            format!("admins only. ask an operator to allowlist this chat ({chat_id}).")
        }
        _ => "unknown command — try /help".to_string(),
    }
}

/// A permissive username check for bot-created users (the API validator is not
/// reachable here); the DB unique constraint still guards duplicates.
fn valid_username(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn blank_update() -> UpdateUser {
    UpdateUser {
        username: None,
        password: None,
        subscription_title: Patch::Missing,
        subscription_description: Patch::Missing,
        subscription_group: Patch::Missing,
        subscription_traffic_policy: None,
        enabled: None,
        traffic_limit_bytes: None,
        expires_at: Patch::Missing,
        device_limit: None,
    }
}

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

async fn audit_bot(
    pool: &PgPool,
    chat_id: i64,
    action: &str,
    user_id: &str,
    details: serde_json::Value,
) {
    let _ = repo::record_audit(
        pool,
        None,
        Some(&format!("telegram:{chat_id}")),
        action,
        "user",
        Some(user_id),
        None,
        details,
    )
    .await;
}

async fn add_user_reply(
    pool: &PgPool,
    args: &[&str],
    public_url: Option<&str>,
    chat_id: i64,
) -> String {
    let username = args.first().copied().unwrap_or("");
    if !valid_username(username) {
        return "usage: /adduser <username> [gb] [days]  (username: letters, digits, . _ -)"
            .to_string();
    }
    let gb: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let days: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let expires_at = if days > 0 {
        Some(chrono::Utc::now() + chrono::Duration::days(days))
    } else {
        None
    };
    let password = match auth::random_token() {
        Ok(p) => p,
        Err(_) => return "rng failed".to_string(),
    };
    let new = NewUser {
        username: username.to_string(),
        password,
        subscription_title: None,
        subscription_description: None,
        subscription_group: None,
        subscription_traffic_policy: "inherit".into(),
        traffic_limit_bytes: (gb.max(0.0) * GB) as i64,
        expires_at,
        device_limit: 0,
    };
    match repo::create_user(pool, new, None, true).await {
        Ok((user, token)) => {
            audit_bot(
                pool,
                chat_id,
                "create",
                &user.id.to_string(),
                serde_json::json!({"via":"telegram","username":user.username}),
            )
            .await;
            let base = public_url.unwrap_or("");
            format!(
                "created {}\nlink: {base}/sub/{token}\napplies on the next node sync",
                user.username
            )
        }
        Err(_) => format!("could not add {username} (already exists?)"),
    }
}

async fn set_quota_reply(pool: &PgPool, args: &[&str], chat_id: i64) -> String {
    let username = args.first().copied().unwrap_or("");
    let Some(gb) = args.get(1).and_then(|s| s.parse::<f64>().ok()) else {
        return "usage: /setquota <username> <gb>  (0 = unlimited)".to_string();
    };
    let Ok(Some(user)) = repo::get_user_by_name(pool, username).await else {
        return format!("no user named {username}");
    };
    let mut upd = blank_update();
    upd.traffic_limit_bytes = Some((gb.max(0.0) * GB) as i64);
    match repo::update_user(pool, user.id, upd).await {
        Ok(Some(_)) => {
            audit_bot(
                pool,
                chat_id,
                "update",
                &user.id.to_string(),
                serde_json::json!({"via":"telegram","traffic_limit_gb":gb}),
            )
            .await;
            format!(
                "set {username} quota to {} — applies on the next node sync",
                if gb > 0.0 {
                    format!("{gb} GB")
                } else {
                    "unlimited".to_string()
                }
            )
        }
        _ => "update failed".to_string(),
    }
}

async fn set_expiry_reply(pool: &PgPool, args: &[&str], chat_id: i64) -> String {
    let username = args.first().copied().unwrap_or("");
    let Some(days) = args.get(1).and_then(|s| s.parse::<i64>().ok()) else {
        return "usage: /setexpiry <username> <days>  (0 = never)".to_string();
    };
    let Ok(Some(user)) = repo::get_user_by_name(pool, username).await else {
        return format!("no user named {username}");
    };
    let mut upd = blank_update();
    upd.expires_at = if days > 0 {
        Patch::Value(chrono::Utc::now() + chrono::Duration::days(days))
    } else {
        Patch::Null
    };
    match repo::update_user(pool, user.id, upd).await {
        Ok(Some(_)) => {
            audit_bot(
                pool,
                chat_id,
                "update",
                &user.id.to_string(),
                serde_json::json!({"via":"telegram","expires_days":days}),
            )
            .await;
            format!(
                "set {username} expiry to {} — applies on the next node sync",
                if days > 0 {
                    format!("{days} day(s)")
                } else {
                    "never".to_string()
                }
            )
        }
        _ => "update failed".to_string(),
    }
}

async fn toggle_reply(pool: &PgPool, username: &str, enable: bool, chat_id: i64) -> String {
    if username.is_empty() {
        return format!(
            "usage: /{} <username>",
            if enable { "enable" } else { "disable" }
        );
    }
    let Ok(Some(user)) = repo::get_user_by_name(pool, username).await else {
        return format!("no user named {username}");
    };
    let mut upd = blank_update();
    upd.enabled = Some(enable);
    match repo::update_user(pool, user.id, upd).await {
        Ok(Some(_)) => {
            audit_bot(
                pool,
                chat_id,
                if enable { "enable" } else { "disable" },
                &user.id.to_string(),
                serde_json::json!({"via":"telegram"}),
            )
            .await;
            format!(
                "{} {username} — applies on the next node sync",
                if enable { "enabled" } else { "disabled" }
            )
        }
        _ => "update failed".to_string(),
    }
}

fn help_text(is_admin: bool, chat_id: i64) -> String {
    let mut lines = vec![
        "honey bot".to_string(),
        "/sub <token> — your subscription link, traffic and expiry".to_string(),
    ];
    if is_admin {
        lines.push("/status — fleet summary".to_string());
        lines.push("/nodes — nodes + reachability".to_string());
        lines.push("/find <username> — a user's quota/expiry".to_string());
        lines.push("/adduser <username> [gb] [days] — create a user + link".to_string());
        lines.push("/setquota <username> <gb> — set traffic limit (0 = ∞)".to_string());
        lines.push("/setexpiry <username> <days> — set expiry (0 = never)".to_string());
        lines.push("/enable <username> · /disable <username>".to_string());
    } else {
        lines.push(format!(
            "(this chat id is {chat_id}; ask an operator for admin access)"
        ));
    }
    lines.join("\n")
}

fn human(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes.max(0) as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

async fn sub_reply(pool: &PgPool, token: &str, public_url: Option<&str>) -> String {
    let Ok(uuid) = token.parse::<Uuid>() else {
        return "usage: /sub <subscription-token>".to_string();
    };
    match repo::get_user_by_subscription_token(pool, uuid).await {
        Ok(Some(user)) => {
            let base = public_url.unwrap_or("");
            let limit = if user.traffic_limit_bytes > 0 {
                human(user.traffic_limit_bytes)
            } else {
                "∞".to_string()
            };
            let expiry = user
                .expires_at
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "never".to_string());
            format!(
                "{}\nlink: {base}/sub/{token}\ntraffic: {} / {}\nexpires: {}\nstatus: {}",
                user.username,
                human(user.used_traffic_bytes),
                limit,
                expiry,
                if user.is_active() {
                    "active"
                } else {
                    "suppressed"
                }
            )
        }
        _ => "no subscription found for that token".to_string(),
    }
}

async fn status_reply(pool: &PgPool) -> String {
    match repo::metrics_snapshot(pool).await {
        Ok((nodes, online, users, active, inbounds, traffic)) => format!(
            "nodes: {online}/{nodes} online\ninbounds: {inbounds}\nusers: {active}/{users} active\ntraffic: {}",
            human(traffic)
        ),
        Err(e) => format!("status failed: {e}"),
    }
}

async fn nodes_reply(pool: &PgPool) -> String {
    match repo::list_nodes(pool).await {
        Ok(nodes) if !nodes.is_empty() => nodes
            .iter()
            .map(|n| {
                let seen = n
                    .last_seen
                    .map(|_| "online")
                    .filter(|_| {
                        n.last_seen
                            .map(|t| (chrono::Utc::now() - t).num_seconds() < 120)
                            .unwrap_or(false)
                    })
                    .unwrap_or("offline");
                format!("• {} ({}) — {seen}", n.name, n.address)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Ok(_) => "no nodes yet".to_string(),
        Err(e) => format!("nodes failed: {e}"),
    }
}

async fn find_reply(pool: &PgPool, username: &str) -> String {
    if username.is_empty() {
        return "usage: /find <username>".to_string();
    }
    match repo::get_user_by_name(pool, username).await {
        Ok(Some(user)) => {
            let limit = if user.traffic_limit_bytes > 0 {
                human(user.traffic_limit_bytes)
            } else {
                "∞".to_string()
            };
            format!(
                "{}\ntraffic: {} / {}\nexpires: {}\nstatus: {}",
                user.username,
                human(user.used_traffic_bytes),
                limit,
                user.expires_at
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "never".to_string()),
                user.suppressed_reason().unwrap_or("active")
            )
        }
        Ok(None) => format!("no user named {username}"),
        Err(e) => format!("find failed: {e}"),
    }
}
