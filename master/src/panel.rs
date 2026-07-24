//! Embedded admin panel and the domain/path allowlist used to expose it.
use anyhow::{bail, Context, Result};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use url::Url;

use crate::api::AppState;
use crate::db::repo;

const INDEX: &str = include_str!("../../web/index.html");
const CSS: &str = include_str!("../../web/app.css");
const JS: &str = include_str!("../../web/app.js");
const FONT: &[u8] = include_bytes!("../../web/assets/PretendardVariable.woff2");
const FONT_LICENSE: &str = include_str!("../../web/assets/PRETENDARD-LICENSE.txt");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelTarget {
    pub host: String,
    pub base_path: String,
}

impl PanelTarget {
    pub fn parse(input: &str, path_override: Option<&str>) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("domain must not be empty");
        }

        let has_scheme = input.contains("://");
        let source = if has_scheme {
            input.to_owned()
        } else {
            format!("https://{input}")
        };
        let url = Url::parse(&source).context("invalid panel domain or URL")?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("panel URL scheme must be http or https");
        }
        if !url.username().is_empty() || url.password().is_some() {
            bail!("panel URL must not contain credentials");
        }
        if url.query().is_some() || url.fragment().is_some() {
            bail!("panel URL must not contain a query or fragment");
        }

        let host = url
            .host_str()
            .context("panel URL has no host")?
            .to_ascii_lowercase();
        if host.contains([':', '/', ' ']) {
            bail!("use a DNS name or IPv4 address without a port");
        }

        let authority_and_path = input.split_once("://").map_or(input, |(_, rest)| rest);
        let supplied_path = authority_and_path
            .find('/')
            .map(|index| &authority_and_path[index..]);
        let raw_path = path_override.or(supplied_path).unwrap_or("/panel");
        let base_path = normalize_path(raw_path)?;
        Ok(Self { host, base_path })
    }

    pub fn url(&self) -> String {
        format!("https://{}{}", self.host, self.base_path)
    }
}

fn normalize_path(input: &str) -> Result<String> {
    let mut path = input.trim().to_owned();
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    if path == "/" {
        bail!("panel must use a non-root path, for example /panel");
    }
    if path.contains("//")
        || path.split('/').any(|part| matches!(part, "." | ".."))
        || !path
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '~'))
    {
        bail!("panel path may contain only URL-safe letters, digits, /, -, _, . and ~");
    }
    Ok(path)
}

pub async fn serve(State(st): State<AppState>, request: Request) -> Response {
    let Some(host) = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(host_without_port)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let host = host.to_ascii_lowercase();
    let path = request.uri().path();
    let domain = match repo::find_panel_domain(&st.pool, &host, path).await {
        Ok(Some(domain)) => domain,
        Ok(None) => {
            tracing::warn!(code = "M1002", %host, "panel denied for host");
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(error) => {
            tracing::error!(code = "M1003", %error, "panel domain lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if path == domain.base_path {
        return Redirect::permanent(&format!("{}/", domain.base_path)).into_response();
    }
    let relative = path
        .strip_prefix(&domain.base_path)
        .unwrap_or(path)
        .trim_start_matches('/');

    match relative {
        "" => asset("text/html; charset=utf-8", INDEX, "no-cache"),
        "app.css" => asset(
            "text/css; charset=utf-8",
            CSS,
            "public, max-age=3600, must-revalidate",
        ),
        "app.js" => asset(
            "text/javascript; charset=utf-8",
            JS,
            "public, max-age=3600, must-revalidate",
        ),
        "assets/PretendardVariable.woff2" => {
            binary_asset("font/woff2", FONT, "public, max-age=31536000, immutable")
        }
        "assets/PRETENDARD-LICENSE.txt" => asset(
            "text/plain; charset=utf-8",
            FONT_LICENSE,
            "public, max-age=31536000, immutable",
        ),
        other if !other.contains('.') => asset("text/html; charset=utf-8", INDEX, "no-cache"),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn host_without_port(authority: &str) -> Option<&str> {
    let authority = authority.trim();
    if authority.is_empty() {
        return None;
    }
    if authority.starts_with('[') {
        return authority
            .strip_prefix('[')?
            .split_once(']')
            .map(|(host, _)| host);
    }
    Some(
        authority
            .split_once(':')
            .map_or(authority, |(host, _)| host),
    )
}

fn asset(content_type: &'static str, content: &'static str, cache: &'static str) -> Response {
    response(content_type, Body::from(content), cache)
}

fn binary_asset(
    content_type: &'static str,
    content: &'static [u8],
    cache: &'static str,
) -> Response {
    response(content_type, Body::from(content), cache)
}

fn response(content_type: &'static str, body: Body, cache: &'static str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache)
        .header("x-content-type-options", "nosniff")
        .header("x-frame-options", "DENY")
        .header("referrer-policy", "same-origin")
        .header(
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
        )
        .body(body)
        .expect("static panel response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_path() {
        assert_eq!(
            PanelTarget::parse("EXAMPLE.com/admin/", None).unwrap(),
            PanelTarget {
                host: "example.com".into(),
                base_path: "/admin".into(),
            }
        );
        assert_eq!(
            PanelTarget::parse("https://panel.example.com", None)
                .unwrap()
                .base_path,
            "/panel"
        );
        assert_eq!(
            PanelTarget::parse("example.com", Some("honey/ui"))
                .unwrap()
                .base_path,
            "/honey/ui"
        );
    }

    #[test]
    fn rejects_unsafe_targets() {
        assert!(PanelTarget::parse("ftp://example.com/panel", None).is_err());
        assert!(PanelTarget::parse("https://user@example.com/panel", None).is_err());
        assert!(PanelTarget::parse("example.com/../admin", None).is_err());
        assert!(PanelTarget::parse("example.com/", None).is_err());
    }

    #[test]
    fn strips_request_ports() {
        assert_eq!(host_without_port("example.com:443"), Some("example.com"));
        assert_eq!(host_without_port("example.com"), Some("example.com"));
        assert_eq!(host_without_port(""), None);
    }
}
