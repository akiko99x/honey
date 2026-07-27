//! Public subscription documents and client configuration generation.
use anyhow::{anyhow, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value as Json};
use url::Url;
use uuid::Uuid;

use crate::db::models::{RoutingProfile, SubscriptionEndpoint, User};

#[derive(Debug, Serialize)]
pub struct EndpointLink {
    pub inbound_id: Uuid,
    pub node: String,
    pub tag: String,
    pub protocol: String,
    pub core: String,
    pub address: String,
    pub port: i32,
    pub uri: Option<String>,
    pub error: Option<String>,
}

/// Duplicate each direct endpoint once per extra node address, so the client
/// gets several failover targets per inbound. CDN-fronted endpoints connect to
/// the transport_host, so extra origin addresses add nothing there.
pub fn expand_endpoints(endpoints: Vec<SubscriptionEndpoint>) -> Vec<SubscriptionEndpoint> {
    let mut out = Vec::new();
    for endpoint in endpoints {
        let fronted = endpoint
            .transport_host
            .as_deref()
            .map(|h| !h.is_empty())
            .unwrap_or(false);
        if fronted || endpoint.extra_addresses.is_empty() {
            out.push(endpoint);
            continue;
        }
        let extras = endpoint.extra_addresses.clone();
        out.push(endpoint.clone());
        for (i, addr) in extras.into_iter().enumerate() {
            let mut alt = endpoint.clone();
            alt.address = addr;
            alt.tag = format!("{}-alt{}", endpoint.tag, i + 1);
            if let Some(name) = alt
                .extra
                .get_mut("happ")
                .and_then(Json::as_object_mut)
                .and_then(|happ| happ.get_mut("name"))
            {
                if let Some(base) = name.as_str() {
                    *name = json!(format!("{base} · alt {}", i + 1));
                }
            }
            out.push(alt);
        }
    }
    out
}

pub fn endpoint_links(user: &User, endpoints: &[SubscriptionEndpoint]) -> Vec<EndpointLink> {
    endpoints
        .iter()
        .map(|endpoint| match endpoint_uri(user, endpoint) {
            Ok(uri) => link(endpoint, Some(uri), None),
            Err(error) => {
                tracing::warn!(code = "M0704", inbound_id = %endpoint.inbound_id, %error, "subscription endpoint could not be rendered");
                link(
                    endpoint,
                    None,
                    Some("unsupported or invalid endpoint configuration".into()),
                )
            }
        })
        .collect()
}

pub fn singbox_client_config(
    user: &User,
    endpoints: &[SubscriptionEndpoint],
    profile: Option<&RoutingProfile>,
) -> Json {
    singbox_config(user, endpoints, false, profile)
}

/// Same proxies, but with a `tun` inbound so the client captures all traffic
/// (system-wide VPN) and routes it through the auto-select group.
pub fn singbox_tun_config(
    user: &User,
    endpoints: &[SubscriptionEndpoint],
    profile: Option<&RoutingProfile>,
) -> Json {
    singbox_config(user, endpoints, true, profile)
}

fn singbox_config(
    user: &User,
    endpoints: &[SubscriptionEndpoint],
    tun: bool,
    profile: Option<&RoutingProfile>,
) -> Json {
    let proxies: Vec<Json> = endpoints
        .iter()
        .filter_map(|endpoint| {
            singbox_outbound(&user.uuid, &user.username, &user.password, endpoint).ok()
        })
        .collect();
    let tags: Vec<String> = proxies
        .iter()
        .filter_map(|o| o.get("tag").and_then(Json::as_str).map(String::from))
        .collect();
    let has_proxy = !tags.is_empty();

    let mut outbounds = Vec::new();
    if has_proxy {
        outbounds.push(json!({
            "type": "urltest", "tag": "auto", "outbounds": tags,
            "url": "https://www.gstatic.com/generate_204", "interval": "5m"
        }));
        let mut select = vec![json!("auto")];
        select.extend(tags.iter().map(|t| json!(t)));
        outbounds.push(
            json!({"type": "selector", "tag": "proxy", "outbounds": select, "default": "auto"}),
        );
    }
    outbounds.extend(proxies);
    outbounds.push(json!({"type": "direct", "tag": "direct"}));

    let final_proxy = profile.map(|p| p.final_proxy).unwrap_or(true);
    let final_tag = if has_proxy && final_proxy {
        "proxy"
    } else {
        "direct"
    };
    let (mut rules, rule_sets, mut need_block) = singbox_rules(profile, has_proxy);

    // client DNS hardening: DoH resolver, optional FakeIP, and a :53 block so
    // plaintext DNS can't leak around the tunnel.
    let dns = profile.and_then(|p| dns_block(p, has_proxy));
    if profile.is_some_and(|p| p.dns_block_plain) {
        need_block = true;
        // put the block rule first so it wins over the proxy/direct final.
        // no "network" → matches both UDP and TCP :53, so plaintext DNS can't
        // leak over either transport around the DoH resolver.
        rules.insert(0, json!({"port": 53, "outbound": "block"}));
    }

    if need_block {
        outbounds.push(json!({"type": "block", "tag": "block"}));
    }

    let mut route = json!({"rules": rules, "final": final_tag});
    if !rule_sets.is_empty() {
        route["rule_set"] = json!(rule_sets);
    }
    let mut config = json!({"log": {"level": "info"}, "outbounds": outbounds, "route": route});
    if let Some(dns) = dns {
        config["dns"] = dns;
    }
    if tun {
        config["route"]["auto_detect_interface"] = json!(true);
        config["inbounds"] = json!([{
            "type": "tun", "tag": "tun-in",
            "address": ["172.19.0.1/30"],
            "auto_route": true, "strict_route": true, "stack": "mixed", "sniff": true
        }]);
    }
    config
}

/// The client-side `dns` block for a profile, or None when no DoH is set. The
/// DoH resolver is reached through the proxy (so lookups are tunnelled); its own
/// hostname is bootstrapped via a local resolver on the direct path. Optional
/// FakeIP routes A/AAAA through a fake pool to prevent DNS-based leaks.
fn dns_block(profile: &RoutingProfile, has_proxy: bool) -> Option<Json> {
    let doh = profile.dns_doh.trim();
    if doh.is_empty() {
        return None;
    }
    let detour = if has_proxy { "proxy" } else { "direct" };
    let mut servers = vec![
        json!({"tag": "remote", "address": doh, "detour": detour, "address_resolver": "local"}),
        json!({"tag": "local", "address": "local", "detour": "direct"}),
    ];
    let mut dns = json!({ "final": "remote" });
    if profile.dns_fakeip {
        servers.push(json!({"tag": "fakeip", "address": "fakeip"}));
        dns["rules"] = json!([{"query_type": ["A", "AAAA"], "server": "fakeip"}]);
        dns["fakeip"] =
            json!({"enabled": true, "inet4_range": "198.18.0.0/15", "inet6_range": "fc00::/18"});
        dns["independent_cache"] = json!(true);
    }
    dns["servers"] = json!(servers);
    Some(dns)
}

fn singbox_rule_set(tag: &str, geo: &str, detour: &str) -> Json {
    let repo = if geo == "geoip" {
        "sing-geoip"
    } else {
        "sing-geosite"
    };
    json!({
        "type": "remote", "tag": tag, "format": "binary",
        "url": format!("https://raw.githubusercontent.com/SagerNet/{repo}/rule-set/{tag}.srs"),
        "download_detour": detour
    })
}

fn singbox_rules(
    profile: Option<&RoutingProfile>,
    has_proxy: bool,
) -> (Vec<Json>, Vec<Json>, bool) {
    let detour = if has_proxy { "proxy" } else { "direct" };
    let mut rules = Vec::new();
    let mut sets = Vec::new();
    let Some(p) = profile else {
        rules.push(json!({"ip_is_private": true, "outbound": "direct"}));
        return (rules, sets, false);
    };
    let mut need_block = false;
    // content-filter / parental: block categories via geosite rule-sets.
    let mut block_category = |code: &str| {
        let tag = format!("geosite-{code}");
        sets.push(singbox_rule_set(&tag, "geosite", detour));
        rules.push(json!({"rule_set": tag, "outbound": "block"}));
        need_block = true;
    };
    if p.block_ads {
        block_category("category-ads-all");
    }
    if p.block_adult {
        block_category("category-porn");
    }
    if p.block_gambling {
        block_category("category-gambling");
    }
    // custom per-profile domain lists (suffix match), blocks first.
    if !p.blocked_domains.is_empty() {
        rules.push(json!({"domain_suffix": p.blocked_domains, "outbound": "block"}));
        need_block = true;
    }
    if p.direct_private {
        rules.push(json!({"ip_is_private": true, "outbound": "direct"}));
    }
    if !p.direct_domains.is_empty() {
        rules.push(json!({"domain_suffix": p.direct_domains, "outbound": "direct"}));
    }
    for code in &p.direct_geosite {
        let tag = format!("geosite-{code}");
        sets.push(singbox_rule_set(&tag, "geosite", detour));
        rules.push(json!({"rule_set": tag, "outbound": "direct"}));
    }
    for code in &p.direct_geoip {
        let tag = format!("geoip-{code}");
        sets.push(singbox_rule_set(&tag, "geoip", detour));
        rules.push(json!({"rule_set": tag, "outbound": "direct"}));
    }
    if has_proxy && !p.proxy_domains.is_empty() {
        rules.push(json!({"domain_suffix": p.proxy_domains, "outbound": "proxy"}));
    }
    // per-app rules: geosite category -> direct / proxy / block.
    for (geosite, action) in parse_app_rules(&p.app_rules) {
        let tag = format!("geosite-{geosite}");
        sets.push(singbox_rule_set(&tag, "geosite", detour));
        let outbound = match action.as_str() {
            "block" => {
                need_block = true;
                "block"
            }
            "proxy" if has_proxy => "proxy",
            "proxy" => "direct",
            _ => "direct",
        };
        rules.push(json!({"rule_set": tag, "outbound": outbound}));
    }
    (rules, sets, need_block)
}

/// Parse a routing profile's `app_rules` json into (geosite, action) pairs.
fn parse_app_rules(value: &Json) -> Vec<(String, String)> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|rule| {
                    let geosite = rule.get("geosite")?.as_str()?.trim().to_string();
                    let action = rule.get("action")?.as_str()?.to_string();
                    if geosite.is_empty()
                        || !matches!(action.as_str(), "direct" | "proxy" | "block")
                    {
                        return None;
                    }
                    Some((geosite, action))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Clash / Mihomo config. Emitted as JSON (valid YAML) so Clash reads it. Proxies
/// + a select group over an auto (url-test) group + rules from the profile.
pub fn clash_config(
    user: &User,
    endpoints: &[SubscriptionEndpoint],
    profile: Option<&RoutingProfile>,
) -> String {
    let mut proxies = Vec::new();
    let mut names = Vec::new();
    for endpoint in endpoints {
        if let Ok((name, proxy)) = clash_proxy(user, endpoint) {
            names.push(name);
            proxies.push(proxy);
        }
    }
    let auto = json!({
        "name": "Auto", "type": "url-test", "proxies": names.clone(),
        "url": "https://www.gstatic.com/generate_204", "interval": 300
    });
    let mut select = vec![json!("Auto")];
    select.extend(names.iter().map(|n| json!(n)));
    let groups = if names.is_empty() {
        json!([])
    } else {
        json!([{"name": "Proxy", "type": "select", "proxies": select}, auto])
    };
    let final_proxy = profile.map(|p| p.final_proxy).unwrap_or(true);
    let target = if !names.is_empty() && final_proxy {
        "Proxy"
    } else {
        "DIRECT"
    };

    let mut rules: Vec<Json> = Vec::new();
    match profile {
        Some(p) => {
            if p.block_ads {
                rules.push(json!("GEOSITE,category-ads-all,REJECT"));
            }
            if p.block_adult {
                rules.push(json!("GEOSITE,category-porn,REJECT"));
            }
            if p.block_gambling {
                rules.push(json!("GEOSITE,category-gambling,REJECT"));
            }
            for domain in &p.blocked_domains {
                rules.push(json!(format!("DOMAIN-SUFFIX,{domain},REJECT")));
            }
            if p.direct_private {
                rules.push(json!("GEOIP,PRIVATE,DIRECT,no-resolve"));
            }
            for domain in &p.direct_domains {
                rules.push(json!(format!("DOMAIN-SUFFIX,{domain},DIRECT")));
            }
            for code in &p.direct_geosite {
                rules.push(json!(format!("GEOSITE,{code},DIRECT")));
            }
            for code in &p.direct_geoip {
                rules.push(json!(format!("GEOIP,{},DIRECT", code.to_uppercase())));
            }
            for domain in &p.proxy_domains {
                rules.push(json!(format!("DOMAIN-SUFFIX,{domain},{target}")));
            }
            for (geosite, action) in parse_app_rules(&p.app_rules) {
                let dest = match action.as_str() {
                    "block" => "REJECT",
                    "proxy" => target,
                    _ => "DIRECT",
                };
                rules.push(json!(format!("GEOSITE,{geosite},{dest}")));
            }
        }
        None => {
            rules.push(json!("GEOIP,LAN,DIRECT,no-resolve"));
            rules.push(json!("GEOIP,PRIVATE,DIRECT,no-resolve"));
        }
    }
    rules.push(json!(format!("MATCH,{target}")));

    let config = json!({
        "mixed-port": 7890,
        "mode": "rule",
        "proxies": proxies,
        "proxy-groups": groups,
        "rules": rules
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

fn clash_proxy(user: &User, e: &SubscriptionEndpoint) -> Result<(String, Json)> {
    let name = client_label(user, e);
    let server = connect_host(e).to_string();
    let network = if e.network.is_empty() {
        "tcp"
    } else {
        e.network.as_str()
    };
    if matches!(e.kind.as_str(), "vless" | "vmess" | "trojan")
        && !matches!(network, "tcp" | "ws" | "httpupgrade" | "grpc")
    {
        return Err(anyhow!(
            "clash transport '{network}' is not supported for '{}'",
            e.kind
        ));
    }
    let mut p = json!({"name": name, "server": server, "port": e.listen_port});

    let apply_tls = |p: &mut Json| {
        p["tls"] = json!(true);
        if let Some(sni) = e.server_name.as_deref() {
            p["servername"] = json!(sni);
        }
        if e.reality {
            if let Some(pbk) = e.reality_public_key.as_deref() {
                let mut ro = json!({"public-key": pbk});
                if let Some(sid) = e.reality_short_ids.first() {
                    ro["short-id"] = json!(sid);
                }
                p["reality-opts"] = ro;
                p["client-fingerprint"] = json!(e.utls_fingerprint.as_deref().unwrap_or("qq"));
            }
        }
    };
    let apply_transport = |p: &mut Json| match network {
        "ws" | "httpupgrade" => {
            p["network"] = json!("ws");
            let mut ws = json!({"path": e.transport_path.clone().unwrap_or_default()});
            if let Some(host) = e.transport_host.as_deref() {
                ws["headers"] = json!({"Host": host});
            }
            p["ws-opts"] = ws;
        }
        "grpc" => {
            p["network"] = json!("grpc");
            p["grpc-opts"] =
                json!({"grpc-service-name": e.transport_service_name.clone().unwrap_or_default()});
        }
        _ => {}
    };

    match e.kind.as_str() {
        "vless" => {
            p["type"] = json!("vless");
            p["uuid"] = json!(user.uuid.to_string());
            if !e.flow.is_empty() {
                p["flow"] = json!(e.flow);
            }
            if e.tls_enabled {
                apply_tls(&mut p);
            }
            apply_transport(&mut p);
        }
        "vmess" => {
            p["type"] = json!("vmess");
            p["uuid"] = json!(user.uuid.to_string());
            p["alterId"] = json!(0);
            p["cipher"] = json!("auto");
            if e.tls_enabled {
                apply_tls(&mut p);
            }
            apply_transport(&mut p);
        }
        "trojan" => {
            p["type"] = json!("trojan");
            p["password"] = json!(user.password);
            if let Some(sni) = e.server_name.as_deref() {
                p["sni"] = json!(sni);
            }
            apply_transport(&mut p);
        }
        "hysteria2" => {
            p["type"] = json!("hysteria2");
            p["password"] = json!(hysteria_auth(user));
            if let Some(sni) = e.server_name.as_deref() {
                p["sni"] = json!(sni);
            }
        }
        "shadowsocks" => {
            let method = e
                .extra
                .get("method")
                .and_then(Json::as_str)
                .ok_or_else(|| anyhow!("shadowsocks needs extra.method"))?;
            p["type"] = json!("ss");
            p["cipher"] = json!(method);
            p["password"] = json!(user.password);
        }
        other => return Err(anyhow!("clash proxy for '{other}' is not supported")),
    }
    Ok((name, p))
}

/// Base64 of newline-joined URIs — the single-URL format v2ray-style
/// subscription clients (v2rayN, NekoBox, Streisand, ...) expect.
pub fn v2ray_document(user: &User, endpoints: &[SubscriptionEndpoint]) -> String {
    let joined = endpoint_links(user, endpoints)
        .into_iter()
        .filter_map(|link| link.uri)
        .collect::<Vec<_>>()
        .join("\n");
    STANDARD.encode(joined)
}

pub fn happ_v2ray_document(user: &User, endpoints: &[SubscriptionEndpoint]) -> String {
    v2ray_document(user, endpoints)
}

/// `Subscription-Userinfo` header value so clients can show quota + expiry.
/// `used_traffic_bytes` is up+down combined, reported here as download.
pub fn userinfo_header(user: &User) -> String {
    let total = user.traffic_limit_bytes.max(0);
    let used = user.used_traffic_bytes.max(0);
    let expire = user.expires_at.map(|at| at.timestamp()).unwrap_or(0);
    format!("upload=0; download={used}; total={total}; expire={expire}")
}

/// Happ and the common XTLS subscription convention accept a UTF-8 title as
/// `base64:<payload>`. Encoding unconditionally also keeps non-ASCII titles out
/// of HTTP's restricted header-value character set.
pub fn profile_title_header(user: &User) -> String {
    let title: String = profile_title(user).chars().take(25).collect();
    format!("base64:{}", STANDARD.encode(title))
}

pub fn profile_title(user: &User) -> &str {
    user.subscription_title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(&user.username)
}

/// Happ's optional announcement metadata. The template is operator-controlled
/// and may contain a few deliberately small, non-secret placeholders.
pub fn announce_header(user: &User) -> Option<String> {
    let template = user
        .subscription_description
        .as_deref()
        .filter(|v| !v.trim().is_empty())?;
    let elapsed = (Utc::now() - user.created_at).num_days().max(0);
    let left = user
        .expires_at
        .map(|at| (at - Utc::now()).num_days().max(0).to_string())
        .unwrap_or_else(|| "∞".to_string());
    let spent = human_bytes(user.used_traffic_bytes.max(0));
    let mut text = template.to_string();
    for (key, value) in [
        ("{USERNAME}", user.username.clone()),
        ("{{USERNAME}}", user.username.clone()),
        ("{DAYS_ELAPSED}", elapsed.to_string()),
        ("{days_elapsed}", elapsed.to_string()),
        ("{DAYS ELAPSED}", elapsed.to_string()),
        ("{{DAYS_ELAPSED}}", elapsed.to_string()),
        ("{TRAFFIC_SPENT}", spent.clone()),
        ("{traffic_spent}", spent),
        (
            "{TRAFFIC SPENT}",
            human_bytes(user.used_traffic_bytes.max(0)),
        ),
        (
            "{{TRAFFIC_SPENT}}",
            human_bytes(user.used_traffic_bytes.max(0)),
        ),
        ("{DAYS_LEFT}", left.clone()),
        ("{days_left}", left.clone()),
        ("{DAYS LEFT}", left.clone()),
        ("{{DAYS_LEFT}}", left),
    ] {
        text = text.replace(key, &value);
    }
    let text: String = text.chars().take(200).collect();
    (!text.trim().is_empty()).then(|| format!("base64:{}", STANDARD.encode(text)))
}

fn human_bytes(value: i64) -> String {
    const UNITS: [&str; 5] = ["B", "GB", "TB", "PB", "EB"];
    let mut n = value as f64;
    let mut index = 0usize;
    while n >= 1024.0 && index < UNITS.len() - 1 {
        n /= 1024.0;
        index += 1;
    }
    if index == 0 {
        format!("{} {}", n as i64, UNITS[index])
    } else {
        format!("{n:.1} {}", UNITS[index])
    }
}

fn link(
    endpoint: &SubscriptionEndpoint,
    uri: Option<String>,
    error: Option<String>,
) -> EndpointLink {
    EndpointLink {
        inbound_id: endpoint.inbound_id,
        node: endpoint.node_name.clone(),
        tag: client_label_from_endpoint(endpoint),
        protocol: endpoint.kind.clone(),
        core: endpoint.core.clone(),
        address: endpoint.address.clone(),
        port: endpoint.listen_port,
        uri,
        error,
    }
}

fn endpoint_uri(user: &User, endpoint: &SubscriptionEndpoint) -> Result<String> {
    match endpoint.kind.as_str() {
        "vless" => vless_uri(user, endpoint),
        "hysteria2" => hysteria2_uri(user, endpoint),
        "trojan" => password_uri("trojan", user, endpoint),
        "tuic" => tuic_uri(user, endpoint),
        "vmess" => vmess_uri(user, endpoint),
        "shadowsocks" => shadowsocks_uri(user, endpoint),
        kind => Err(anyhow!(
            "client link for protocol '{kind}' is not supported yet"
        )),
    }
}

fn vless_uri(user: &User, endpoint: &SubscriptionEndpoint) -> Result<String> {
    let mut url = credential_url("vless", &user.uuid.to_string(), None, endpoint)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("encryption", "none");
        if !endpoint.flow.is_empty() {
            query.append_pair("flow", &endpoint.flow);
        }
    }
    append_security(&mut url, endpoint)?;
    append_transport(&mut url, endpoint);
    set_label(&mut url, user, endpoint);
    Ok(url.into())
}

/// Hysteria2 UDP port-hopping range from the inbound's extra (`"20000-30000"`).
fn hop_ports(endpoint: &SubscriptionEndpoint) -> Option<String> {
    endpoint
        .extra
        .get("hop_ports")
        .and_then(Json::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn hysteria2_uri(user: &User, endpoint: &SubscriptionEndpoint) -> Result<String> {
    if !endpoint.tls_enabled {
        return Err(anyhow!("hysteria2 requires TLS"));
    }

    let auth = hysteria_auth(user);
    let mut url = credential_url("hysteria2", &auth, None, endpoint)?;
    // Keep the canonical slash before the query. Some third-party importers
    // reject the otherwise valid empty-path URL.
    url.set_path("/");
    if let Some(server_name) = endpoint.server_name.as_deref() {
        url.query_pairs_mut().append_pair("sni", server_name);
    }
    if let Some(ports) = hop_ports(endpoint) {
        url.query_pairs_mut().append_pair("mport", &ports);
    }
    set_label(&mut url, user, endpoint);
    Ok(url.into())
}

fn hysteria_auth(user: &User) -> String {
    format!("{}:{}", user.username, user.password)
}

fn password_uri(scheme: &str, user: &User, endpoint: &SubscriptionEndpoint) -> Result<String> {
    let mut url = credential_url(scheme, &user.password, None, endpoint)?;
    append_security(&mut url, endpoint)?;
    append_transport(&mut url, endpoint);
    set_label(&mut url, user, endpoint);
    Ok(url.into())
}

fn tuic_uri(user: &User, endpoint: &SubscriptionEndpoint) -> Result<String> {
    let mut url = credential_url(
        "tuic",
        &user.uuid.to_string(),
        Some(&user.password),
        endpoint,
    )?;
    append_security(&mut url, endpoint)?;
    url.query_pairs_mut()
        .append_pair("congestion_control", "bbr");
    set_label(&mut url, user, endpoint);
    Ok(url.into())
}

fn vmess_uri(user: &User, endpoint: &SubscriptionEndpoint) -> Result<String> {
    let security = if endpoint.tls_enabled { "tls" } else { "" };
    let network = if endpoint.network.is_empty() {
        "tcp"
    } else {
        endpoint.network.as_str()
    };
    // vmess json: grpc carries serviceName in `path`; others carry path/host.
    let (net, path, host) = match network {
        "grpc" => (
            "grpc",
            endpoint.transport_service_name.clone().unwrap_or_default(),
            String::new(),
        ),
        "h2" => (
            "h2",
            endpoint.transport_path.clone().unwrap_or_default(),
            endpoint.transport_host.clone().unwrap_or_default(),
        ),
        other => (
            other,
            endpoint.transport_path.clone().unwrap_or_default(),
            endpoint.transport_host.clone().unwrap_or_default(),
        ),
    };
    let document = json!({
        "v": "2",
        "ps": label(user, endpoint),
        "add": connect_host(endpoint),
        "port": endpoint.listen_port.to_string(),
        "id": user.uuid,
        "aid": "0",
        "scy": "auto",
        "net": net,
        "type": "none",
        "host": host,
        "path": path,
        "tls": security,
        "sni": endpoint.server_name.as_deref().unwrap_or("")
    });
    Ok(format!(
        "vmess://{}",
        STANDARD.encode(serde_json::to_vec(&document)?)
    ))
}

fn shadowsocks_uri(user: &User, endpoint: &SubscriptionEndpoint) -> Result<String> {
    let method = endpoint
        .extra
        .get("method")
        .and_then(Json::as_str)
        .ok_or_else(|| anyhow!("shadowsocks inbound extra.method is required for a client link"))?;
    let credential = URL_SAFE_NO_PAD.encode(format!("{method}:{}", user.password));
    let host = display_host(connect_host(endpoint));
    let fragment =
        url::form_urlencoded::byte_serialize(label(user, endpoint).as_bytes()).collect::<String>();
    // SIP003 plugin (obfs / v2ray-plugin / cloak): carried as ?plugin=<name;opts>.
    let query = match ss_plugin(endpoint) {
        Some((name, opts)) => {
            let spec = if opts.is_empty() {
                name
            } else {
                format!("{name};{opts}")
            };
            let enc = url::form_urlencoded::byte_serialize(spec.as_bytes()).collect::<String>();
            format!("?plugin={enc}")
        }
        None => String::new(),
    };
    Ok(format!(
        "ss://{credential}@{host}:{}{query}#{fragment}",
        endpoint.listen_port
    ))
}

/// SIP003 plugin (name, opts) from an inbound's extra, if configured.
fn ss_plugin(endpoint: &SubscriptionEndpoint) -> Option<(String, String)> {
    let name = endpoint
        .extra
        .get("plugin")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let opts = endpoint
        .extra
        .get("plugin_opts")
        .and_then(Json::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Some((name.to_string(), opts))
}

fn credential_url(
    scheme: &str,
    username: &str,
    password: Option<&str>,
    endpoint: &SubscriptionEndpoint,
) -> Result<Url> {
    let host = display_host(connect_host(endpoint));
    let mut url = Url::parse(&format!("{scheme}://{host}:{}", endpoint.listen_port))?;
    url.set_username(username)
        .map_err(|_| anyhow!("invalid username for {scheme} link"))?;
    url.set_password(password)
        .map_err(|_| anyhow!("invalid password for {scheme} link"))?;
    Ok(url)
}

fn append_security(url: &mut Url, endpoint: &SubscriptionEndpoint) -> Result<()> {
    let mut query = url.query_pairs_mut();
    if endpoint.reality {
        let public_key = endpoint
            .reality_public_key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| anyhow!("reality_public_key is required for a client link"))?;
        query.append_pair("security", "reality");
        query.append_pair("pbk", public_key);
        if let Some(server_name) = endpoint.server_name.as_deref() {
            query.append_pair("sni", server_name);
        }
        if let Some(short_id) = endpoint.reality_short_ids.first() {
            query.append_pair("sid", short_id);
        }
    } else if endpoint.tls_enabled {
        query.append_pair("security", "tls");
        if let Some(server_name) = endpoint.server_name.as_deref() {
            query.append_pair("sni", server_name);
        }
    } else {
        query.append_pair("security", "none");
    }
    Ok(())
}

/// Appends the network transport (`type=...` + params) and the uTLS fingerprint
/// to a v2ray-style URL (vless / trojan).
fn append_transport(url: &mut Url, endpoint: &SubscriptionEndpoint) {
    let mut query = url.query_pairs_mut();
    let network = if endpoint.network.is_empty() {
        "tcp"
    } else {
        endpoint.network.as_str()
    };
    query.append_pair("type", network);
    match network {
        "ws" | "httpupgrade" | "http" | "h2" => {
            if let Some(path) = endpoint.transport_path.as_deref() {
                query.append_pair("path", path);
            }
            if let Some(host) = endpoint.transport_host.as_deref() {
                query.append_pair("host", host);
            }
        }
        "grpc" => {
            if let Some(sn) = endpoint.transport_service_name.as_deref() {
                query.append_pair("serviceName", sn);
            }
        }
        "xhttp" => {
            if let Some(path) = endpoint.transport_path.as_deref() {
                query.append_pair("path", path);
            }
            if let Some(host) = endpoint.transport_host.as_deref() {
                query.append_pair("host", host);
            }
            if let Some(mode) = endpoint.transport_mode.as_deref() {
                query.append_pair("mode", mode);
            }
        }
        _ => {}
    }
    if endpoint.tls_enabled || endpoint.reality {
        let fp = endpoint
            .utls_fingerprint
            .as_deref()
            .filter(|f| !f.is_empty())
            .unwrap_or("qq");
        query.append_pair("fp", fp);
    }
}

pub(crate) fn singbox_outbound(
    uuid: &str,
    username: &str,
    password: &str,
    endpoint: &SubscriptionEndpoint,
) -> Result<Json> {
    let tag = client_label_from_endpoint(endpoint);
    let server = connect_host(endpoint);
    let mut outbound = match endpoint.kind.as_str() {
        "vless" => json!({
            "type": "vless", "tag": tag, "server": server,
            "server_port": endpoint.listen_port, "uuid": uuid, "flow": endpoint.flow
        }),
        "vmess" => json!({
            "type": "vmess", "tag": tag, "server": server,
            "server_port": endpoint.listen_port, "uuid": uuid, "security": "auto"
        }),
        "hysteria2" => json!({
            "type": "hysteria2", "tag": tag, "server": server,
            "server_port": endpoint.listen_port,
            "password": format!("{username}:{password}")
        }),
        "trojan" => json!({
            "type": "trojan", "tag": tag, "server": server,
            "server_port": endpoint.listen_port, "password": password
        }),
        "tuic" => json!({
            "type": "tuic", "tag": tag, "server": server,
            "server_port": endpoint.listen_port, "uuid": uuid,
            "password": password, "congestion_control": "bbr"
        }),
        "shadowsocks" => {
            let mut ss = json!({
                "type": "shadowsocks", "tag": tag, "server": server,
                "server_port": endpoint.listen_port,
                "method": endpoint.extra.get("method").and_then(Json::as_str)
                    .ok_or_else(|| anyhow!("shadowsocks extra.method is required"))?,
                "password": password
            });
            if let Some((name, opts)) = ss_plugin(endpoint) {
                ss["plugin"] = json!(name);
                ss["plugin_opts"] = json!(opts);
            }
            ss
        }
        kind => return Err(anyhow!("sing-box outbound for '{kind}' is not supported")),
    };
    if endpoint.kind == "hysteria2" {
        if let Some(ports) = hop_ports(endpoint) {
            outbound["server_ports"] = json!([ports.replace('-', ":")]);
        }
    }
    if endpoint.tls_enabled {
        let mut tls = json!({
            "enabled": true,
            "server_name": endpoint.server_name.as_deref().unwrap_or(&endpoint.address)
        });
        if endpoint.reality {
            tls["reality"] = json!({
                "enabled": true,
                "public_key": endpoint.reality_public_key,
                "short_id": endpoint.reality_short_ids.first()
            });
        }
        let fingerprint = endpoint
            .utls_fingerprint
            .as_deref()
            .filter(|value| !value.is_empty())
            .or(endpoint.reality.then_some("qq"));
        if let Some(fingerprint) = fingerprint {
            tls["utls"] = json!({"enabled": true, "fingerprint": fingerprint});
        }
        if endpoint.ech {
            tls["ech"] = json!({"enabled": true});
        }
        outbound["tls"] = tls;
    }
    if let Some(transport) = singbox_transport(endpoint)? {
        outbound["transport"] = transport;
    }
    Ok(outbound)
}

/// sing-box client-side transport block (mirror of the server side).
fn singbox_transport(endpoint: &SubscriptionEndpoint) -> Result<Option<Json>> {
    let transport = match endpoint.network.as_str() {
        "" | "tcp" => None,
        "ws" => {
            let mut t = json!({ "type": "ws" });
            if let Some(path) = endpoint.transport_path.as_deref() {
                t["path"] = json!(path);
            }
            if let Some(host) = endpoint.transport_host.as_deref() {
                t["headers"] = json!({ "Host": host });
            }
            Some(t)
        }
        "grpc" => Some(json!({
            "type": "grpc",
            "service_name": endpoint.transport_service_name.clone().unwrap_or_default()
        })),
        "http" | "h2" => {
            let mut t = json!({ "type": "http" });
            if let Some(path) = endpoint.transport_path.as_deref() {
                t["path"] = json!(path);
            }
            if let Some(host) = endpoint.transport_host.as_deref() {
                t["host"] = json!([host]);
            }
            Some(t)
        }
        "httpupgrade" => {
            let mut t = json!({ "type": "httpupgrade" });
            if let Some(path) = endpoint.transport_path.as_deref() {
                t["path"] = json!(path);
            }
            if let Some(host) = endpoint.transport_host.as_deref() {
                t["host"] = json!(host);
            }
            Some(t)
        }
        "quic" => Some(json!({ "type": "quic" })),
        "xhttp" | "mkcp" => {
            return Err(anyhow!(
                "sing-box client transport '{}' is not supported",
                endpoint.network
            ));
        }
        other => return Err(anyhow!("unknown sing-box client transport '{other}'")),
    };
    Ok(transport)
}

/// The host a client should connect to. For a CDN-fronted inbound (a v2ray
/// transport carrying a transport_host) that's the CDN hostname, not the origin
/// IP — so the client hits the CDN and the origin IP stays hidden.
fn connect_host(endpoint: &SubscriptionEndpoint) -> &str {
    match endpoint.transport_host.as_deref() {
        Some(host)
            if !host.is_empty()
                && matches!(
                    endpoint.network.as_str(),
                    "ws" | "http" | "h2" | "httpupgrade" | "xhttp"
                ) =>
        {
            host
        }
        _ => endpoint.address.as_str(),
    }
}

fn display_host(address: &str) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]")
    } else {
        address.to_string()
    }
}

fn set_label(url: &mut Url, user: &User, endpoint: &SubscriptionEndpoint) {
    url.set_fragment(Some(&label(user, endpoint)));
}

fn label(user: &User, endpoint: &SubscriptionEndpoint) -> String {
    let title = client_label(user, endpoint);
    match happ_value(endpoint, "description") {
        Some(description) => format!(
            "{title}?serverDescription={}",
            STANDARD.encode(description.as_bytes())
        ),
        None => title,
    }
}

fn client_label(user: &User, endpoint: &SubscriptionEndpoint) -> String {
    let fallback = format!(
        "{} @ {} / {}",
        user.username, endpoint.node_name, endpoint.tag
    );
    client_label_with_fallback(endpoint, &fallback)
}

fn client_label_from_endpoint(endpoint: &SubscriptionEndpoint) -> String {
    let fallback = format!("{}-{}", endpoint.node_name, endpoint.tag);
    client_label_with_fallback(endpoint, &fallback)
}

fn client_label_with_fallback(endpoint: &SubscriptionEndpoint, fallback: &str) -> String {
    let name = happ_value(endpoint, "name").unwrap_or(fallback);
    match happ_value(endpoint, "country_code").and_then(country_flag) {
        Some(flag) => format!("{flag} {name}"),
        None => name.to_string(),
    }
}

fn happ_value<'a>(endpoint: &'a SubscriptionEndpoint, key: &str) -> Option<&'a str> {
    endpoint
        .extra
        .get("happ")
        .and_then(|value| value.get(key))
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn country_flag(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();
    if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    Some(
        code.bytes()
            .filter_map(|byte| char::from_u32(0x1F1E6 + u32::from(byte - b'A')))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn user() -> User {
        User {
            id: Uuid::new_v4(),
            username: "alice".into(),
            subscription_title: None,
            subscription_description: None,
            labels: vec![],
            uuid: Uuid::new_v4().to_string(),
            password: "secret".into(),
            enabled: true,
            traffic_limit_bytes: 0,
            used_traffic_bytes: 0,
            expires_at: None,
            device_limit: 0,
            routing_profile_id: None,
            quota_interval: "none".into(),
            quota_reset_at: None,
            created_by: None,
            subscription_alias: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn dns_profile(doh: &str, fakeip: bool, block: bool) -> RoutingProfile {
        RoutingProfile {
            id: Uuid::new_v4(),
            name: "p".into(),
            version: 1,
            block_ads: false,
            direct_private: true,
            direct_geosite: vec![],
            direct_geoip: vec![],
            final_proxy: true,
            is_default: false,
            notes: String::new(),
            block_adult: false,
            block_gambling: false,
            blocked_domains: vec![],
            direct_domains: vec![],
            proxy_domains: vec![],
            app_rules: json!([]),
            dns_doh: doh.into(),
            dns_fakeip: fakeip,
            dns_block_plain: block,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn dns_hardening_emits_doh_fakeip_and_block() {
        // no DoH -> no dns section, no block rule
        let plain = dns_profile("", false, false);
        assert!(dns_block(&plain, true).is_none());
        let cfg = singbox_client_config(&user(), &[endpoint()], Some(&plain));
        assert!(cfg.get("dns").is_none());

        // DoH + fakeip + block :53
        let hardened = dns_profile("https://dns.quad9.net/dns-query", true, true);
        let dns = dns_block(&hardened, true).expect("dns block");
        let servers = dns["servers"].as_array().unwrap();
        assert!(servers
            .iter()
            .any(|s| s["address"] == "https://dns.quad9.net/dns-query"));
        assert!(servers.iter().any(|s| s["address"] == "fakeip"));
        assert_eq!(dns["fakeip"]["enabled"], json!(true));

        let cfg = singbox_client_config(&user(), &[endpoint()], Some(&hardened));
        assert_eq!(cfg["dns"]["final"], json!("remote"));
        // the :53 block rule is present and there is a block outbound.
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert!(rules
            .iter()
            .any(|r| r["port"] == json!(53) && r["outbound"] == "block"));
        assert!(cfg["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["tag"] == "block"));
    }

    fn endpoint() -> SubscriptionEndpoint {
        SubscriptionEndpoint {
            inbound_id: Uuid::new_v4(),
            node_name: "ams-1".into(),
            address: "vpn.example.com".into(),
            tag: "vless-in".into(),
            kind: "vless".into(),
            core: "singbox".into(),
            listen_port: 443,
            flow: "xtls-rprx-vision".into(),
            tls_enabled: true,
            server_name: Some("example.com".into()),
            reality: true,
            reality_public_key: Some("public-key".into()),
            reality_short_ids: vec!["deadbeef".into()],
            network: "tcp".into(),
            transport_path: None,
            transport_host: None,
            transport_service_name: None,
            transport_mode: None,
            utls_fingerprint: None,
            ech: false,
            extra: json!({}),
            extra_addresses: vec![],
        }
    }

    fn outbound_by_tag<'a>(config: &'a Json, tag: &str) -> &'a Json {
        config["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|outbound| outbound["tag"] == tag)
            .unwrap()
    }

    #[test]
    fn builds_reality_vless_link_and_client_config() {
        let user = user();
        let endpoint = endpoint();
        let uri = endpoint_uri(&user, &endpoint).unwrap();
        assert!(uri.starts_with("vless://"));
        assert!(uri.contains("security=reality"));
        assert!(uri.contains("pbk=public-key"));
        let config = singbox_client_config(&user, &[endpoint], None);
        let outbound = outbound_by_tag(&config, "ams-1-vless-in");
        assert_eq!(outbound["type"], "vless");
        assert_eq!(outbound["tls"]["reality"]["public_key"], "public-key");
    }

    #[test]
    fn emits_happ_title_flag_name_and_description() {
        let mut user = user();
        user.subscription_title = Some("Мой VPN".into());
        let mut endpoint = endpoint();
        endpoint.extra = json!({
            "happ": {
                "name": "Premium",
                "country_code": "PL",
                "description": "low latency"
            }
        });
        let uri = endpoint_uri(&user, &endpoint).unwrap();
        assert!(uri.contains("%F0%9F%87%B5%F0%9F%87%B1%20Premium"));
        assert!(uri.contains("serverDescription=bG93IGxhdGVuY3k="));
        assert_eq!(profile_title_header(&user), "base64:0JzQvtC5IFZQTg==");
        let config = singbox_client_config(&user, &[endpoint], None);
        assert_eq!(outbound_by_tag(&config, "🇵🇱 Premium")["tag"], "🇵🇱 Premium");
    }

    #[test]
    fn renders_subscription_announcement_tags() {
        let mut user = user();
        user.subscription_description =
            Some("Hi {USERNAME}: used {TRAFFIC_SPENT}, left {DAYS_LEFT}".into());
        let header = announce_header(&user).expect("announcement");
        assert!(header.starts_with("base64:"));
        let decoded = STANDARD
            .decode(header.trim_start_matches("base64:"))
            .unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert!(text.contains("Hi alice"));
        assert!(text.contains("left ∞"));
    }

    #[test]
    fn builds_canonical_hysteria2_uri() {
        let user = user();
        let mut endpoint = endpoint();
        endpoint.kind = "hysteria2".into();
        endpoint.address = "203.0.113.10".into();
        endpoint.reality = false;
        endpoint.server_name = Some("203.0.113.10".into());

        let uri = endpoint_uri(&user, &endpoint).unwrap();
        assert!(uri.starts_with("hysteria2://alice%3Asecret@203.0.113.10:443/?sni=203.0.113.10"));
        assert!(!uri.contains("security="));

        let parsed = Url::parse(&uri).unwrap();
        assert_eq!(parsed.path(), "/");
        assert_eq!(
            parsed
                .query_pairs()
                .find(|(key, _)| key == "sni")
                .unwrap()
                .1,
            "203.0.113.10"
        );

        let config = singbox_client_config(&user, &[endpoint.clone()], None);
        let outbound = outbound_by_tag(&config, "ams-1-vless-in");
        assert_eq!(outbound["type"], "hysteria2");
        assert_eq!(outbound["server"], "203.0.113.10");
        assert_eq!(outbound["password"], "alice:secret");

        let clash: Json = serde_json::from_str(&clash_config(&user, &[endpoint], None)).unwrap();
        assert_eq!(clash["proxies"][0]["type"], "hysteria2");
        assert_eq!(clash["proxies"][0]["password"], "alice:secret");
    }

    #[test]
    fn renders_happ_hysteria_userpass_in_uri_auth() {
        let user = user();
        let mut endpoint = endpoint();
        endpoint.kind = "hysteria2".into();
        endpoint.address = "hy2.example.com".into();
        endpoint.reality = false;
        endpoint.server_name = Some("hy2.example.com".into());

        let encoded = happ_v2ray_document(&user, &[endpoint]);
        let decoded = String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap();
        assert!(decoded
            .starts_with("hysteria2://alice%3Asecret@hy2.example.com:443/?sni=hy2.example.com"));
        assert!(!decoded.contains("&auth="));
    }

    #[test]
    fn carries_transport_utls_and_ech_into_clients() {
        let user = user();
        let mut endpoint = endpoint();
        endpoint.reality = false;
        endpoint.network = "ws".into();
        endpoint.transport_path = Some("/honey".into());
        endpoint.transport_host = Some("cdn.example.com".into());
        endpoint.utls_fingerprint = Some("firefox".into());
        endpoint.ech = true;

        let uri = endpoint_uri(&user, &endpoint).unwrap();
        assert!(uri.contains("type=ws"));
        assert!(uri.contains("path=%2Fhoney"));
        assert!(uri.contains("fp=firefox"));

        let config = singbox_client_config(&user, &[endpoint], None);
        let outbound = outbound_by_tag(&config, "ams-1-vless-in");
        assert_eq!(outbound["server"], "cdn.example.com");
        assert_eq!(outbound["transport"]["type"], "ws");
        assert_eq!(outbound["tls"]["utls"]["fingerprint"], "firefox");
        assert_eq!(outbound["tls"]["ech"]["enabled"], true);
    }

    #[test]
    fn uses_cdn_host_in_all_supported_client_outputs() {
        let user = user();
        let mut endpoint = endpoint();
        endpoint.reality = false;
        endpoint.network = "ws".into();
        endpoint.transport_path = Some("/edge".into());
        endpoint.transport_host = Some("cdn.example.com".into());

        let uri = endpoint_uri(&user, &endpoint).unwrap();
        assert!(uri.contains("@cdn.example.com:443"));
        assert!(uri.contains("host=cdn.example.com"));

        let clash: Json =
            serde_json::from_str(&clash_config(&user, &[endpoint.clone()], None)).unwrap();
        assert_eq!(clash["proxies"][0]["server"], "cdn.example.com");

        for config in [
            singbox_client_config(&user, &[endpoint.clone()], None),
            singbox_tun_config(&user, &[endpoint], None),
        ] {
            assert_eq!(
                outbound_by_tag(&config, "ams-1-vless-in")["server"],
                "cdn.example.com"
            );
        }
    }

    #[test]
    fn emits_xhttp_uri_but_omits_unsupported_client_formats() {
        let user = user();
        let mut endpoint = endpoint();
        endpoint.core = "xray".into();
        endpoint.reality = false;
        endpoint.flow.clear();
        endpoint.network = "xhttp".into();
        endpoint.transport_path = Some("/honey".into());
        endpoint.transport_host = Some("cdn.example.com".into());
        endpoint.transport_mode = Some("auto".into());

        let uri = endpoint_uri(&user, &endpoint).unwrap();
        assert!(uri.contains("@cdn.example.com:443"));
        assert!(uri.contains("type=xhttp"));
        assert!(uri.contains("path=%2Fhoney"));
        assert!(uri.contains("mode=auto"));

        let singbox = singbox_client_config(&user, &[endpoint.clone()], None);
        assert!(singbox["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|outbound| outbound["type"] != "vless"));
        let clash: Json = serde_json::from_str(&clash_config(&user, &[endpoint], None)).unwrap();
        assert!(clash["proxies"].as_array().unwrap().is_empty());
    }
}
