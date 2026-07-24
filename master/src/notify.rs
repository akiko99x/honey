//! Outbound notifications: fan alerts out to configured channels (generic
//! webhook, Discord/Slack incoming webhooks, or a Telegram bot). Sends are
//! best-effort and deduped per key with a cooldown so a persistently-down node
//! doesn't spam every reconcile tick.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use sqlx::PgPool;

use crate::db::{models::NotifyChannel, repo};

const COOLDOWN: Duration = Duration::from_secs(1800); // 30 min per key

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default()
    })
}

fn should_send_fallback(key: &str) -> bool {
    static SEEN: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let map = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    guard.retain(|_, t| now.duration_since(*t) < Duration::from_secs(86_400));
    match guard.get(key) {
        Some(last) if now.duration_since(*last) < COOLDOWN => false,
        _ => {
            guard.insert(key.to_string(), now);
            true
        }
    }
}

fn metadata(event: &str) -> (&'static str, &'static str, Option<&'static str>) {
    match event {
        "node_down" => ("critical", "M0409", Some("node")),
        "push_failed" => ("critical", "M0406", Some("node")),
        "cert_expiry" => ("warning", "M1301", Some("domain")),
        "quota_reset" => ("info", "M1401", Some("user")),
        "subscription_abuse" => ("warning", "M1701", Some("subscription")),
        "traffic_anomaly" => ("warning", "M1610", Some("user")),
        "device_limit" => ("warning", "M1611", Some("user")),
        "config_drift" => ("warning", "M1612", Some("node")),
        _ => ("warning", "M1600", None),
    }
}

fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Persist an alert and fan newly-created events out to external channels.
/// Database deduplication survives process restarts; the in-memory limiter is
/// retained only as a safe fallback when persistence itself is unavailable.
pub async fn alert(
    pool: &PgPool,
    event: &str,
    key: &str,
    title: &str,
    body: &str,
    resource_id: &str,
) {
    let (severity, code, resource_type) = metadata(event);
    let key = bounded(key, 256);
    let title = bounded(title, 160);
    let body = bounded(body, 1024);
    let resource_id = bounded(resource_id, 128);
    let should_dispatch = match repo::record_system_notification(
        pool,
        event,
        &key,
        severity,
        code,
        &title,
        &body,
        resource_type,
        Some(&resource_id),
    )
    .await
    {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                code = "M1604",
                "in-app notification persistence failed: {error:#}"
            );
            should_send_fallback(&key)
        }
    };
    if should_dispatch {
        dispatch(pool, event, &title, &body).await;
    }
}

/// Enforce the retention bound even during long quiet periods with no new
/// alerts. The foreign-key cascade removes per-admin read markers with events.
pub async fn retention(pool: PgPool, interval: Duration) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        // HA: singleton loop — only the lease holder acts.
        if !crate::ha::is_leader() {
            continue;
        }
        if let Err(error) = repo::prune_system_notifications(&pool).await {
            tracing::warn!(code = "M1605", "notification retention failed: {error:#}");
        }
    }
}

/// Send to every matching channel (no dedup — used by the manual test button).
pub async fn dispatch(pool: &PgPool, event: &str, title: &str, body: &str) {
    let channels = match repo::channels_for_event(pool, event).await {
        Ok(channels) => channels,
        Err(e) => {
            tracing::warn!(code = "M1602", "notify: channel lookup failed: {e:#}");
            return;
        }
    };
    for channel in channels {
        let (title, body) = (title.to_string(), body.to_string());
        tokio::spawn(async move {
            if let Err(e) = send(&channel, &title, &body).await {
                tracing::warn!(code = "M1601", channel = %channel.name, "notify send failed: {e:#}");
            }
        });
    }
}

pub async fn send(channel: &NotifyChannel, title: &str, body: &str) -> Result<()> {
    let response = match channel.kind.as_str() {
        "discord" => {
            client()
                .post(&channel.target)
                .json(&serde_json::json!({ "content": format!("**{title}**\n{body}") }))
                .send()
                .await?
        }
        "slack" => {
            client()
                .post(&channel.target)
                .json(&serde_json::json!({ "text": format!("*{title}*\n{body}") }))
                .send()
                .await?
        }
        "telegram" => {
            // target is "<bot_token>@<chat_id>"; bot tokens never contain '@'.
            let (token, chat_id) = channel
                .target
                .rsplit_once('@')
                .ok_or_else(|| anyhow!("telegram target must be <bot_token>@<chat_id>"))?;
            client()
                .post(format!("https://api.telegram.org/bot{token}/sendMessage"))
                .json(
                    &serde_json::json!({ "chat_id": chat_id, "text": format!("{title}\n{body}") }),
                )
                .send()
                .await?
        }
        "email" => email_response(&channel.target, title, body).await?,
        "sms" => sms_response(&channel.target, title, body).await?,
        "alertmanager" => alertmanager_response(&channel.target, title, body).await?,
        // generic webhook
        _ => {
            client()
                .post(&channel.target)
                .json(&serde_json::json!({ "title": title, "body": body }))
                .send()
                .await?
        }
    };
    response.error_for_status()?;
    Ok(())
}

/// Email over an HTTP provider API (no SMTP dependency). Target:
/// `resend|<api_key>|<from>|<to>` or `mailgun|<domain>|<api_key>|<from>|<to>`.
async fn email_response(target: &str, title: &str, body: &str) -> Result<reqwest::Response> {
    match target.split('|').collect::<Vec<_>>().as_slice() {
        ["resend", key, from, to] => Ok(client()
            .post("https://api.resend.com/emails")
            .bearer_auth(key)
            .json(&serde_json::json!({"from": from, "to": [to], "subject": title, "text": body}))
            .send()
            .await?),
        ["mailgun", domain, key, from, to] => Ok(client()
            .post(format!("https://api.mailgun.net/v3/{domain}/messages"))
            .basic_auth("api", Some(key))
            .form(&[
                ("from", *from),
                ("to", *to),
                ("subject", title),
                ("text", body),
            ])
            .send()
            .await?),
        _ => Err(anyhow!(
            "email target must be resend|<key>|<from>|<to> or mailgun|<domain>|<key>|<from>|<to>"
        )),
    }
}

/// SMS via Twilio. Target: `twilio|<account_sid>|<auth_token>|<from>|<to>`.
async fn sms_response(target: &str, title: &str, body: &str) -> Result<reqwest::Response> {
    match target.split('|').collect::<Vec<_>>().as_slice() {
        ["twilio", sid, token, from, to] => {
            let text = format!("{title}\n{body}");
            Ok(client()
                .post(format!(
                    "https://api.twilio.com/2010-04-01/Accounts/{sid}/Messages.json"
                ))
                .basic_auth(sid, Some(token))
                .form(&[("From", *from), ("To", *to), ("Body", text.as_str())])
                .send()
                .await?)
        }
        _ => Err(anyhow!(
            "sms target must be twilio|<account_sid>|<auth_token>|<from>|<to>"
        )),
    }
}

/// Push an alert into Prometheus Alertmanager. Target is the Alertmanager base
/// URL (e.g. http://alertmanager:9093).
async fn alertmanager_response(target: &str, title: &str, body: &str) -> Result<reqwest::Response> {
    let url = target.trim_end_matches('/');
    Ok(client()
        .post(format!("{url}/api/v2/alerts"))
        .json(&serde_json::json!([{
            "labels": {"alertname": "honey", "severity": "warning", "instance": "honey"},
            "annotations": {"summary": title, "description": body}
        }]))
        .send()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_metadata_is_stable_and_safe() {
        assert_eq!(metadata("node_down"), ("critical", "M0409", Some("node")));
        assert_eq!(metadata("quota_reset"), ("info", "M1401", Some("user")));
        assert_eq!(
            metadata("subscription_abuse"),
            ("warning", "M1701", Some("subscription"))
        );
        assert_eq!(metadata("unknown"), ("warning", "M1600", None));
        assert_eq!(bounded("abcdef", 3), "abc");
        assert_eq!(bounded("🐝honey", 2), "🐝h");
    }
}
