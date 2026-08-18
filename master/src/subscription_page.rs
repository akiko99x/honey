//! Public, read-only subscription dashboard and its embedded static assets.
use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use qrcode::render::svg;
use qrcode::QrCode;
use uuid::Uuid;

use crate::db::models::User;
use crate::subscription::EndpointLink;

const TEMPLATE: &str = include_str!("../../web/subscription.html");
const CSS: &str = include_str!("../../web/subscription.css");
const JS: &str = include_str!("../../web/subscription.js");
const FONT: &[u8] = include_bytes!("../../web/assets/PretendardVariable.woff2");

pub async fn css() -> Response {
    text_asset("text/css; charset=utf-8", CSS, "public, max-age=3600")
}

pub async fn js() -> Response {
    text_asset("text/javascript; charset=utf-8", JS, "public, max-age=3600")
}

pub async fn font() -> Response {
    let mut response = Response::new(Body::from(FONT));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("font/woff2"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
}

fn text_asset(content_type: &'static str, body: &'static str, cache: &'static str) -> Response {
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

pub fn render(
    user: &User,
    token: Uuid,
    links: &[EndpointLink],
    fallback_base_url: Option<&str>,
    update_interval_hours: i64,
) -> String {
    let status = user.suppressed_reason().unwrap_or("active");
    let status_class = if status == "active" { "on" } else { "off" };
    let total = user.traffic_limit_bytes.max(0);
    let used = user.used_traffic_bytes.max(0);
    let percent = if total == 0 {
        0
    } else {
        ((used.saturating_mul(100) / total).clamp(0, 100)) as u32
    };
    let remaining = if total == 0 {
        "no traffic limit".to_string()
    } else {
        format!("{} remaining", bytes(total.saturating_sub(used)))
    };
    let traffic_text = if total == 0 {
        format!("{} / unlimited", bytes(used))
    } else {
        format!("{} / {}", bytes(used), bytes(total))
    };
    let metrics = [
        metric("subscription id", &user.id.to_string(), true),
        metric("started", &date(user.created_at), false),
        metric(
            "expires",
            &user.expires_at.map(date).unwrap_or_else(|| "never".into()),
            false,
        ),
        metric("access", &format!("{} endpoints", links.len()), false),
    ]
    .join("");
    let endpoint_rows = if links.is_empty() {
        "<div class=\"empty\" data-i18n=\"No accessible endpoints are available yet.\">No accessible endpoints are available yet.</div>".into()
    } else {
        links
            .iter()
            .map(|link| endpoint(token, link))
            .collect::<Vec<_>>()
            .join("")
    };
    let base = format!("/sub/{token}");

    TEMPLATE
        .replace(
            "{{SUBSCRIPTION_BASE_URL}}",
            &escape(fallback_base_url.unwrap_or_default()),
        )
        .replace(
            "{{UPDATE_INTERVAL_HOURS}}",
            &update_interval_hours.clamp(1, 168).to_string(),
        )
        .replace(
            "{{TITLE}}",
            &escape(crate::subscription::profile_title(user)),
        )
        .replace(
            "{{USERNAME}}",
            &escape(crate::subscription::profile_title(user)),
        )
        .replace("{{STATUS_CLASS}}", status_class)
        .replace("{{STATUS}}", &escape(status))
        .replace("{{METRICS}}", &metrics)
        .replace("{{TRAFFIC_TEXT}}", &escape(&traffic_text))
        .replace("{{TRAFFIC_PERCENT}}", &percent.to_string())
        .replace("{{TRAFFIC_USED}}", &escape(&bytes(used)))
        .replace("{{TRAFFIC_REMAINING}}", &escape(&remaining))
        .replace("{{QR_URL}}", &format!("{base}/qr"))
        .replace("{{V2RAY_URL}}", &format!("{base}/v2ray"))
        .replace("{{SINGBOX_URL}}", &format!("{base}/sing-box"))
        .replace("{{SINGBOX_TUN_URL}}", &format!("{base}/sing-box-tun"))
        .replace("{{CLASH_URL}}", &format!("{base}/clash"))
        .replace("{{LINKS_URL}}", &format!("{base}/links"))
        .replace("{{ENDPOINT_COUNT}}", &links.len().to_string())
        .replace("{{ENDPOINTS}}", &endpoint_rows)
}

fn metric(label: &str, value: &str, code: bool) -> String {
    let tag = if code { "code" } else { "b" };
    format!(
        "<div class=\"metric\"><small data-i18n=\"{0}\">{0}</small><{tag} title=\"{1}\">{1}</{tag}></div>",
        escape(label),
        escape(value)
    )
}

fn endpoint(token: Uuid, link: &EndpointLink) -> String {
    let title = format!("{} / {}", link.node, link.tag);
    match link.uri.as_deref() {
        Some(uri) => format!(
            "<article class=\"endpoint\"><div class=\"protocol\">{protocol}</div><div class=\"endpoint-main\"><b>{title}</b><small>{address}:{port} · {core}</small></div><div class=\"endpoint-actions\"><button data-copy=\"{uri}\" data-i18n=\"copy\">copy</button><a href=\"/sub/{token}/qr/{id}\" target=\"_blank\" rel=\"noopener\" data-i18n=\"qr\">qr</a></div></article>",
            protocol = escape(&link.protocol),
            title = escape(&title),
            address = escape(&link.address),
            port = link.port,
            core = escape(&link.core),
            uri = escape(uri),
            id = link.inbound_id,
        ),
        None => format!(
            "<article class=\"endpoint error\"><div class=\"protocol\">{}</div><div class=\"endpoint-main\"><b>{}</b><small>{}</small></div></article>",
            escape(&link.protocol),
            escape(&title),
            escape(link.error.as_deref().unwrap_or("link unavailable"))
        ),
    }
}

pub fn qr_svg(value: &str) -> anyhow::Result<String> {
    let code = QrCode::new(value.as_bytes())?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(280, 280)
        .dark_color(svg::Color("#050505"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

fn date(value: DateTime<Utc>) -> String {
    value.format("%d %b %Y").to_string()
}

fn bytes(value: i64) -> String {
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

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn html_response(body: String) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_and_escapes_user_content() {
        assert_eq!(bytes(1_073_741_824), "1.0 GB");
        assert_eq!(escape("<x>&\""), "&lt;x&gt;&amp;&quot;");
        assert!(qr_svg("vless://example").unwrap().contains("<svg"));
    }

    #[test]
    fn happ_import_is_a_native_encoded_deep_link() {
        assert!(TEMPLATE.contains("<a class=\"import\" data-import=\"happ\""));
        assert!(JS.contains("happ://add/${enc(happUrl)}"));
        assert!(TEMPLATE.contains("data-subscription-base=\"{{SUBSCRIPTION_BASE_URL}}\""));
    }
}
