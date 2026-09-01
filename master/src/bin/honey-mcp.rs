//! Local stdio MCP bridge for the complete Honey panel API.
//!
//! Authentication is delegated to Honey's normal bearer API keys, so MCP
//! calls receive the same RBAC checks and audit/request IDs as panel calls.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use clap::Parser;
use reqwest::{Method, Url};
use rmcp::{
    handler::server::wrapper::Parameters, schemars, tool, tool_handler, tool_router,
    transport::stdio, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const API_SOURCE: &str = include_str!("../api/mod.rs");
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1_048_576;
const ABSOLUTE_MAX_RESPONSE_BYTES: usize = 10_485_760;

#[derive(Debug, Parser)]
#[command(
    name = "honey-mcp",
    version,
    about = "MCP bridge for the Honey VPN panel"
)]
struct Args {
    /// Honey panel root, including a path prefix when configured.
    #[arg(long, env = "HONEY_BASE_URL")]
    base_url: String,

    /// Named Honey API key. Prefer --api-key-file so the token is not stored in
    /// process arguments or MCP configuration.
    #[arg(long, env = "HONEY_API_KEY", conflicts_with = "api_key_file")]
    api_key: Option<String>,

    /// Root-only/user-only file containing a named Honey API key.
    #[arg(long, env = "HONEY_API_KEY_FILE", conflicts_with = "api_key")]
    api_key_file: Option<PathBuf>,

    #[arg(long, env = "HONEY_MCP_TIMEOUT_SECONDS", default_value_t = 60)]
    timeout_seconds: u64,

    #[arg(
        long,
        env = "HONEY_MCP_MAX_RESPONSE_BYTES",
        default_value_t = DEFAULT_MAX_RESPONSE_BYTES
    )]
    max_response_bytes: usize,
}

#[derive(Debug, Clone)]
struct HoneyMcp {
    client: reqwest::Client,
    base_url: Url,
    api_key: String,
    max_response_bytes: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DiscoverParams {
    /// Case-insensitive filter matched against method, path and category.
    #[serde(default)]
    search: Option<String>,
    /// Optional HTTP method filter: GET, POST, PUT, PATCH or DELETE.
    #[serde(default)]
    method: Option<String>,
    /// Maximum operations returned (1-250).
    #[serde(default = "default_discover_limit")]
    limit: usize,
}

fn default_discover_limit() -> usize {
    100
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadParams {
    /// Panel API path, for example /nodes or /nodes/<uuid>/metrics.
    path: String,
    /// Query parameters. Scalars and arrays of scalars are supported.
    #[serde(default)]
    query: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WriteParams {
    /// POST, PUT or PATCH. DELETE is intentionally a separate tool.
    method: String,
    /// Panel API path, for example /nodes/<uuid>/push.
    path: String,
    /// Query parameters. Scalars and arrays of scalars are supported.
    #[serde(default)]
    query: BTreeMap<String, Value>,
    /// JSON request body. Omit for bodyless actions.
    #[serde(default)]
    body: Option<Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeleteParams {
    /// Panel API path to delete.
    path: String,
    /// Query parameters. Scalars and arrays of scalars are supported.
    #[serde(default)]
    query: BTreeMap<String, Value>,
    /// Must exactly equal `DELETE <path>` to prevent accidental deletion.
    confirm: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StatusParams {
    /// Include the potentially larger metrics and live-connections responses.
    #[serde(default)]
    detailed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct Operation {
    method: String,
    path: String,
    category: String,
    required_role: String,
    destructive: bool,
}

#[derive(Debug, Serialize)]
struct ApiResponse {
    ok: bool,
    method: String,
    path: String,
    status: u16,
    content_type: Option<String>,
    request_id: Option<String>,
    body: Value,
    truncated: bool,
}

#[tool_router]
impl HoneyMcp {
    #[tool(
        name = "honey_discover",
        description = "List every operation exposed by the Honey panel. Use this first when the exact path or method is unknown. The catalog is derived from the panel router, including operations not yet detailed in OpenAPI."
    )]
    async fn discover(&self, Parameters(params): Parameters<DiscoverParams>) -> String {
        let search = params.search.unwrap_or_default().to_ascii_lowercase();
        let method = params.method.map(|value| value.to_ascii_uppercase());
        let limit = params.limit.clamp(1, 250);
        let operations: Vec<_> = route_catalog()
            .into_iter()
            .filter(|operation| {
                method
                    .as_ref()
                    .is_none_or(|wanted| operation.method == *wanted)
                    && (search.is_empty()
                        || format!(
                            "{} {} {}",
                            operation.method, operation.path, operation.category
                        )
                        .to_ascii_lowercase()
                        .contains(&search))
            })
            .take(limit)
            .collect();
        pretty(&json!({
            "ok": true,
            "count": operations.len(),
            "operations": operations,
            "notes": {
                "read": "Use honey_read for GET operations",
                "write": "Use honey_write for POST, PUT and PATCH operations",
                "delete": "Use honey_delete with an exact confirmation string",
                "schema": "GET /openapi.json returns detailed schemas for the stable automation subset"
            }
        }))
    }

    #[tool(
        name = "honey_status",
        description = "Read Honey control-plane, fleet and VPN status without changing state. Returns liveness, readiness, public status, issues and nodes; detailed mode also includes metrics and live connections."
    )]
    async fn status(&self, Parameters(params): Parameters<StatusParams>) -> String {
        let mut paths = vec!["/health", "/ready", "/status", "/issues", "/nodes"];
        if params.detailed {
            paths.extend(["/metrics", "/live-connections"]);
        }
        let mut results = BTreeMap::new();
        for path in paths {
            let response = self
                .request(Method::GET, path, &BTreeMap::new(), None)
                .await;
            results.insert(path, response);
        }
        pretty(&json!({"ok": true, "results": results}))
    }

    #[tool(
        name = "honey_read",
        description = "Perform a read-only GET against any Honey panel endpoint, including nodes, inbounds, users, subscriptions, configuration previews, logs, analytics, reachability and status surfaces."
    )]
    async fn read(&self, Parameters(params): Parameters<ReadParams>) -> String {
        pretty(
            &self
                .request(Method::GET, &params.path, &params.query, None)
                .await,
        )
    }

    #[tool(
        name = "honey_write",
        description = "Perform a state-changing POST, PUT or PATCH against any Honey panel endpoint. This can create or update nodes, users, inbounds, routing, settings and trigger operational actions such as push, probe, rotate, update or restore. Inspect current state first."
    )]
    async fn write(&self, Parameters(params): Parameters<WriteParams>) -> String {
        let method = match params.method.to_ascii_uppercase().as_str() {
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "PATCH" => Method::PATCH,
            value => {
                return tool_error(format!(
                    "method {value:?} is not allowed; use POST, PUT or PATCH"
                ))
            }
        };
        pretty(
            &self
                .request(method, &params.path, &params.query, params.body)
                .await,
        )
    }

    #[tool(
        name = "honey_delete",
        description = "Delete a Honey resource. The confirm field must exactly equal `DELETE <path>`. Read the resource first and use only after explicit user authorization."
    )]
    async fn delete(&self, Parameters(params): Parameters<DeleteParams>) -> String {
        let expected = format!("DELETE {}", params.path);
        if params.confirm != expected {
            return tool_error(format!("confirmation mismatch; expected {expected:?}"));
        }
        pretty(
            &self
                .request(Method::DELETE, &params.path, &params.query, None)
                .await,
        )
    }
}

#[tool_handler(
    name = "honey",
    version = "0.1.5",
    instructions = "Manage the configured Honey VPN panel. Discover or read current state before writes. Treat honey_write as state-changing and honey_delete as destructive; never delete without explicit user authorization."
)]
impl ServerHandler for HoneyMcp {}

impl HoneyMcp {
    fn new(args: Args) -> Result<Self> {
        let mut base_url = Url::parse(&args.base_url).context("invalid HONEY_BASE_URL")?;
        if !matches!(base_url.scheme(), "http" | "https") {
            anyhow::bail!("HONEY_BASE_URL must use http or https");
        }
        if !base_url.username().is_empty() || base_url.password().is_some() {
            anyhow::bail!("HONEY_BASE_URL must not contain credentials");
        }
        if base_url.host_str().is_none()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            anyhow::bail!("HONEY_BASE_URL must be an absolute URL without query or fragment");
        }
        if base_url.scheme() == "http" && !is_loopback_host(base_url.host_str().unwrap_or_default())
        {
            anyhow::bail!("plain HTTP is allowed only for localhost or a loopback address");
        }
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }

        let api_key = match (args.api_key, args.api_key_file) {
            (Some(value), None) => value,
            (None, Some(path)) => read_api_key_file(&path)?,
            (None, None) => anyhow::bail!("set HONEY_API_KEY or HONEY_API_KEY_FILE"),
            (Some(_), Some(_)) => unreachable!("clap enforces conflicts_with"),
        };
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            anyhow::bail!("Honey API key is empty");
        }
        if args.max_response_bytes == 0 || args.max_response_bytes > ABSOLUTE_MAX_RESPONSE_BYTES {
            anyhow::bail!("max response bytes must be between 1 and {ABSOLUTE_MAX_RESPONSE_BYTES}");
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(args.timeout_seconds.clamp(1, 600)))
            .user_agent(format!("honey-mcp/{}", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            base_url,
            api_key,
            max_response_bytes: args.max_response_bytes,
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &BTreeMap<String, Value>,
        body: Option<Value>,
    ) -> Value {
        let normalized = match normalize_path(path) {
            Ok(value) => value,
            Err(error) => return json!({"ok": false, "error": error.to_string()}),
        };
        let mut url = match self.base_url.join(normalized.trim_start_matches('/')) {
            Ok(value) => value,
            Err(error) => return json!({"ok": false, "error": format!("invalid path: {error}")}),
        };
        if let Err(error) = append_query(&mut url, query) {
            return json!({"ok": false, "error": error.to_string()});
        }

        let mut request = self
            .client
            .request(method.clone(), url)
            .bearer_auth(&self.api_key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = match request.send().await {
            Ok(value) => value,
            Err(error) => {
                return json!({
                    "ok": false,
                    "method": method.as_str(),
                    "path": normalized,
                    "error": format!("Honey request failed: {error}")
                })
            }
        };
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or(0)
                .min(self.max_response_bytes as u64) as usize,
        );
        let mut truncated = false;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let remaining = self.max_response_bytes.saturating_sub(bytes.len());
                    if chunk.len() > remaining {
                        bytes.extend_from_slice(&chunk[..remaining]);
                        truncated = true;
                        break;
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => {
                    return json!({
                        "ok": false,
                        "method": method.as_str(),
                        "path": normalized,
                        "status": status.as_u16(),
                        "request_id": request_id,
                        "error": format!("could not read Honey response: {error}")
                    })
                }
            }
        }
        let body = decode_body(&bytes, content_type.as_deref());
        serde_json::to_value(ApiResponse {
            ok: status.is_success(),
            method: method.as_str().to_string(),
            path: normalized,
            status: status.as_u16(),
            content_type,
            request_id,
            body,
            truncated,
        })
        .unwrap_or_else(|error| json!({"ok": false, "error": error.to_string()}))
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn read_api_key_file(path: &std::path::Path) -> Result<String> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("could not inspect API key file {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("API key path {} is not a regular file", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            anyhow::bail!(
                "API key file {} must not be readable or writable by group/others",
                path.display()
            );
        }
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("could not read API key file {}", path.display()))
}

fn normalize_path(path: &str) -> Result<String> {
    let path = path.trim();
    if !path.starts_with('/') || path.starts_with("//") {
        anyhow::bail!("path must start with one slash");
    }
    if path.contains('?') || path.contains('#') || path.contains("://") {
        anyhow::bail!("put query parameters in the query object, not in path");
    }
    if path.split('/').any(|segment| matches!(segment, "." | "..")) {
        anyhow::bail!("dot path segments are not allowed");
    }
    Ok(path.to_string())
}

fn append_query(url: &mut Url, query: &BTreeMap<String, Value>) -> Result<()> {
    let mut pairs = url.query_pairs_mut();
    for (key, value) in query {
        match value {
            Value::Array(values) => {
                for value in values {
                    pairs.append_pair(key, &query_scalar(value)?);
                }
            }
            value => {
                pairs.append_pair(key, &query_scalar(value)?);
            }
        }
    }
    Ok(())
}

fn query_scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null => Ok(String::new()),
        _ => anyhow::bail!("query values must be scalars or arrays of scalars"),
    }
}

fn decode_body(bytes: &[u8], content_type: Option<&str>) -> Value {
    if content_type.is_some_and(|value| value.contains("json")) {
        serde_json::from_slice(bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        Value::String(text.to_string())
    } else {
        json!({
            "encoding": "base64",
            "data": base64::engine::general_purpose::STANDARD.encode(bytes)
        })
    }
}

fn route_catalog() -> Vec<Operation> {
    let protected = API_SOURCE
        .split_once("let protected = Router::new()")
        .map(|(_, value)| value)
        .and_then(|value| value.split_once(".route_layer(").map(|(routes, _)| routes))
        .unwrap_or_default();
    let mut operations = Vec::new();
    let mut rest = protected;
    while let Some(start) = rest.find(".route(") {
        rest = &rest[start + ".route(".len()..];
        let Some((call, after)) = balanced_call(rest) else {
            break;
        };
        rest = after;
        let Some(path) = first_string(call) else {
            continue;
        };
        for method in route_methods(call) {
            operations.push(Operation {
                destructive: method == "DELETE",
                required_role: required_role(method, &path).to_string(),
                category: path_category(&path).to_string(),
                method: method.to_string(),
                path: path.clone(),
            });
        }
    }
    operations.sort_by(|left, right| (&left.path, &left.method).cmp(&(&right.path, &right.method)));
    operations.dedup_by(|left, right| left.path == right.path && left.method == right.method);
    operations
}

fn balanced_call(input: &str) -> Option<(&str, &str)> {
    let mut depth = 1usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&input[..index], &input[index + ch.len_utf8()..]));
                }
            }
            _ => {}
        }
    }
    None
}

fn first_string(input: &str) -> Option<String> {
    let start = input.find('"')? + 1;
    let tail = &input[start..];
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

fn route_methods(call: &str) -> Vec<&'static str> {
    let mut methods = Vec::new();
    for (needle, method) in [
        ("get(", "GET"),
        ("post(", "POST"),
        ("routing::put(", "PUT"),
        (".put(", "PUT"),
        ("routing::patch(", "PATCH"),
        (".patch(", "PATCH"),
        ("routing::delete(", "DELETE"),
        (".delete(", "DELETE"),
    ] {
        if call.contains(needle) && !methods.contains(&method) {
            methods.push(method);
        }
    }
    methods
}

fn path_category(path: &str) -> &'static str {
    let path = path.trim_start_matches('/');
    if path.starts_with("nodes")
        || path.starts_with("certificates")
        || path.starts_with("enrollments")
        || path.starts_with("reality")
        || path.starts_with("wireguard")
        || path.starts_with("services")
    {
        "nodes"
    } else if path.starts_with("inbounds") {
        "inbounds"
    } else if path.starts_with("users") || path.starts_with("import") {
        "users"
    } else if path.starts_with("groups") {
        "groups"
    } else if path.starts_with("routing-profiles") {
        "routing"
    } else if path.starts_with("domains") {
        "domains"
    } else if path.starts_with("config") || path.starts_with("scheduled-ops") {
        "config"
    } else if path.starts_with("notify-channels")
        || path.starts_with("telegram")
        || path.starts_with("announcements")
    {
        "notifications"
    } else if path.starts_with("audit") {
        "audit"
    } else if path.starts_with("settings")
        || path.starts_with("branding")
        || path.starts_with("api-keys")
        || path.starts_with("admins")
        || path.starts_with("custom-roles")
        || path.starts_with("admin-ips")
    {
        "admin"
    } else {
        "dashboard"
    }
}

fn required_role(method: &str, path: &str) -> &'static str {
    if path.starts_with("/notifications")
        || path.starts_with("/auth/sessions")
        || path == "/auth/login-history"
        || path.starts_with("/saved-views")
    {
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
    if method == "GET" || path == "/auth/logout" {
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

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|error| tool_error(error.to_string()))
}

fn tool_error(message: impl Into<String>) -> String {
    pretty_fallback(&json!({"ok": false, "error": message.into()}))
}

fn pretty_fallback(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{\"ok\":false}".to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = HoneyMcp::new(Args::parse())?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_the_complete_protected_router() {
        let routes = route_catalog();
        assert!(
            routes.len() >= 150,
            "only {} operations found",
            routes.len()
        );
        for expected in [
            ("GET", "/nodes/:id/metrics"),
            ("POST", "/nodes/:id/push"),
            ("PUT", "/inbounds/:id/reachability"),
            ("PATCH", "/settings"),
            ("DELETE", "/users/:id/subscriptions/:sid"),
            ("POST", "/update/apply"),
        ] {
            assert!(
                routes
                    .iter()
                    .any(|route| route.method == expected.0 && route.path == expected.1),
                "missing {} {}",
                expected.0,
                expected.1
            );
        }
    }

    #[test]
    fn paths_cannot_escape_the_configured_panel_origin() {
        assert_eq!(normalize_path("/nodes").unwrap(), "/nodes");
        for invalid in [
            "nodes",
            "//attacker.invalid/x",
            "/../secret",
            "/nodes?x=1",
            "https://attacker.invalid/",
        ] {
            assert!(normalize_path(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn unencrypted_http_is_loopback_only() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("panel.example.com"));
    }

    #[test]
    fn query_values_are_encoded_without_accepting_nested_objects() {
        assert_eq!(query_scalar(&json!(42)).unwrap(), "42");
        assert_eq!(query_scalar(&json!(true)).unwrap(), "true");
        assert!(query_scalar(&json!({"nested": true})).is_err());
    }

    #[test]
    fn route_roles_match_the_panel_ladder() {
        assert_eq!(required_role("GET", "/nodes"), "viewer");
        assert_eq!(required_role("POST", "/nodes/id/push"), "operator");
        assert_eq!(required_role("DELETE", "/nodes/id"), "admin");
        assert_eq!(required_role("POST", "/update/apply"), "owner");
    }
}
