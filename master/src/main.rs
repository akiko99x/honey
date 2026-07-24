//! honey master.
//!
//! subcommands:
//!   run      — long-lived service: rest api + reconcile loop (+ dial acceptor)
//!   serve    — rest api only
//!   dial     — dial-mode acceptor only (needs --features dial-acceptor)
//!   push     — one-shot: build a node's spec and apply it (serve mode)
//!   ping     — dial an agent over mTLS, run whoru + ping
//!   migrate  — apply Postgres migrations

#[cfg(feature = "acme")]
mod acme;
mod agent_client;
mod api;
mod auth;
mod db;
mod domains;
mod geo;
mod ha;
#[cfg(feature = "tls")]
mod https;
mod issues;
mod logbuf;
mod monitor;
mod notify;
mod panel;
mod pb;
mod pki;
mod quota;
mod ratelimit;
mod reach;
mod reality;
mod reconcile;
mod registry;
mod schedule;
mod secret;
mod secret_source;
mod spec;
mod stats;
mod subscription;
mod subscription_page;
mod telegram;
mod tls;
#[cfg(feature = "dial-acceptor")]
mod tunnel;
mod update;
mod wg;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
#[cfg_attr(not(feature = "tls"), allow(unused_imports))]
use axum::Router;
use clap::{Parser, Subcommand};

use agent_client::AgentClient;
use registry::Registry;

#[derive(Parser, Debug)]
#[command(name = "honey-master", about = "honey master")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// dial an agent and run whoru + ping
    Ping {
        /// agent endpoint, e.g. https://203.0.113.10:8443
        #[arg(long, default_value = "https://127.0.0.1:8443")]
        agent: String,
        /// dir holding ca.crt / master.crt / master.key
        #[arg(long, env = "HONEY_CERTS_DIR", default_value = "./certs")]
        certs_dir: PathBuf,
    },
    /// apply database migrations
    Migrate {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    /// print a fresh base64 master key for HONEY_SECRET_KEY
    Keygen,
    /// (re-)encrypt stored secrets with the current HONEY_SECRET_KEY
    Reencrypt {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    /// rotate stored secrets from HONEY_SECRET_KEY_OLD to HONEY_SECRET_KEY
    Rekey {
        #[arg(long, env = "HONEY_SECRET_KEY_OLD", hide_env_values = true)]
        old_key: String,
        #[arg(long, env = "HONEY_SECRET_KEY", hide_env_values = true)]
        new_key: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    /// manage human panel administrators and roles
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// allow, list or remove a host/path that serves the admin panel
    Domain {
        #[command(subcommand)]
        action: DomainAction,
    },
    /// build a node's spec from the db and push it to its agent (serve mode)
    Push {
        /// node name (from the nodes table)
        node: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long, env = "HONEY_CERTS_DIR", default_value = "./certs")]
        certs_dir: PathBuf,
    },
    /// run the rest api
    Serve {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long, env = "HONEY_CERTS_DIR", default_value = "./certs")]
        certs_dir: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8080")]
        listen: String,
        /// bearer token for the api; empty = auth disabled (localhost only!)
        #[arg(long, env = "HONEY_API_TOKEN")]
        api_token: Option<String>,
        /// serve https from this cert (PEM); needs --tls-key and --features tls
        #[arg(long, env = "HONEY_TLS_CERT")]
        tls_cert: Option<PathBuf>,
        #[arg(long, env = "HONEY_TLS_KEY")]
        tls_key: Option<PathBuf>,
        /// obtain certs via built-in ACME for this domain (repeatable); needs
        /// --features acme and the listen port reachable on :443
        #[arg(long)]
        acme_domain: Vec<String>,
        #[arg(long, env = "HONEY_ACME_EMAIL")]
        acme_email: Option<String>,
        #[arg(long, env = "HONEY_ACME_CACHE", default_value = "/var/lib/honey/acme")]
        acme_cache: PathBuf,
        /// use the Let's Encrypt staging directory (for testing)
        #[arg(long)]
        acme_staging: bool,
    },
    /// accept agents that dial in (NAT nodes). needs --features dial-acceptor
    Dial {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long, env = "HONEY_CERTS_DIR", default_value = "./certs")]
        certs_dir: PathBuf,
        #[arg(long, default_value = "0.0.0.0:9443")]
        listen: String,
    },
    /// long-lived service: rest api + dial acceptor + reconcile loop together
    Run {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long, env = "HONEY_CERTS_DIR", default_value = "./certs")]
        certs_dir: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8080")]
        api_listen: String,
        #[arg(long, default_value = "0.0.0.0:9443")]
        dial_listen: String,
        #[arg(long, env = "HONEY_API_TOKEN")]
        api_token: Option<String>,
        /// how often the reconcile loop runs, seconds
        #[arg(long, default_value_t = 30)]
        reconcile_secs: u64,
        /// serve https from this cert (PEM); needs --tls-key and --features tls
        #[arg(long, env = "HONEY_TLS_CERT")]
        tls_cert: Option<PathBuf>,
        #[arg(long, env = "HONEY_TLS_KEY")]
        tls_key: Option<PathBuf>,
        /// obtain certs via built-in ACME for this domain (repeatable); needs
        /// --features acme and the listen port reachable on :443
        #[arg(long)]
        acme_domain: Vec<String>,
        #[arg(long, env = "HONEY_ACME_EMAIL")]
        acme_email: Option<String>,
        #[arg(long, env = "HONEY_ACME_CACHE", default_value = "/var/lib/honey/acme")]
        acme_cache: PathBuf,
        /// use the Let's Encrypt staging directory (for testing)
        #[arg(long)]
        acme_staging: bool,
    },
}

#[derive(Subcommand, Debug)]
enum DomainAction {
    /// add or re-enable a panel URL, e.g. panel.example.com/honey
    Add {
        target: String,
        /// override the path contained in target (default: /panel)
        #[arg(long)]
        path: Option<String>,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    /// list panel URLs accepted by the master
    List {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    /// remove an accepted panel URL
    Remove {
        target: String,
        #[arg(long)]
        path: Option<String>,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
}

#[derive(Subcommand, Debug)]
enum AdminAction {
    Add {
        username: String,
        #[arg(long, default_value = "owner")]
        role: String,
        #[arg(long, env = "HONEY_ADMIN_PASSWORD", hide_env_values = true)]
        password: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    List {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    Password {
        username: String,
        #[arg(long, env = "HONEY_ADMIN_PASSWORD", hide_env_values = true)]
        password: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    Role {
        username: String,
        role: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    Enable {
        username: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    Disable {
        username: String,
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // fmt layer to stdout/journald + a capture layer feeding the in-memory ring
    // the panel reads via GET /system/logs. one env filter drives both. set
    // HONEY_LOG_FORMAT=json for structured logs (levels: error/warn/info/debug/
    // trace via RUST_LOG, default info).
    use tracing_subscriber::prelude::*;
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let base = tracing_subscriber::registry()
        .with(filter)
        .with(logbuf::layer());
    if std::env::var("HONEY_LOG_FORMAT").ok().as_deref() == Some("json") {
        base.with(tracing_subscriber::fmt::layer().json()).init();
    } else {
        base.with(tracing_subscriber::fmt::layer()).init();
    }

    // load the at-rest encryption key (if any) before anything touches the db.
    // the key may come from env, a mounted file, HashiCorp Vault or a command —
    // see the secret_source module for precedence.
    // geo table for the usage heatmap (built-in special-use ranges + optional
    // operator country CSV via HONEY_GEOIP_FILE).
    geo::init();

    let resolved = secret_source::resolve().await?;
    secret::init(resolved.key.as_deref())?;
    if secret::is_enabled() {
        tracing::info!(
            code = "M0114",
            backend = resolved.backend,
            "at-rest encryption enabled (key backend: {})",
            resolved.backend
        );
    } else {
        tracing::warn!(
            "HONEY_SECRET_KEY not set — user/reality secrets are stored in PLAINTEXT (dev only)"
        );
    }

    match Cli::parse().cmd {
        Cmd::Ping { agent, certs_dir } => ping(&agent, &certs_dir).await,
        Cmd::Migrate { database_url } => migrate(&database_url).await,
        Cmd::Keygen => {
            println!("{}", secret::generate_key_b64()?);
            Ok(())
        }
        Cmd::Reencrypt { database_url } => reencrypt(&database_url).await,
        Cmd::Rekey {
            old_key,
            new_key,
            database_url,
        } => rekey(&old_key, &new_key, &database_url).await,
        Cmd::Admin { action } => admin(action).await,
        Cmd::Domain { action } => domain(action).await,
        Cmd::Push {
            node,
            database_url,
            certs_dir,
        } => push(&node, &database_url, &certs_dir).await,
        Cmd::Serve {
            database_url,
            certs_dir,
            listen,
            api_token,
            tls_cert,
            tls_key,
            acme_domain,
            acme_email,
            acme_cache,
            acme_staging,
        } => {
            let mode = build_tls_mode(
                tls_cert,
                tls_key,
                acme_domain,
                acme_email,
                acme_cache,
                acme_staging,
            )?;
            serve(&database_url, certs_dir, &listen, api_token, mode).await
        }
        Cmd::Dial {
            database_url,
            certs_dir,
            listen,
        } => dial_accept(&database_url, certs_dir, &listen).await,
        Cmd::Run {
            database_url,
            certs_dir,
            api_listen,
            dial_listen,
            api_token,
            reconcile_secs,
            tls_cert,
            tls_key,
            acme_domain,
            acme_email,
            acme_cache,
            acme_staging,
        } => {
            let mode = build_tls_mode(
                tls_cert,
                tls_key,
                acme_domain,
                acme_email,
                acme_cache,
                acme_staging,
            )?;
            run_service(
                &database_url,
                certs_dir,
                api_listen,
                dial_listen,
                api_token,
                reconcile_secs,
                mode,
            )
            .await
        }
    }
}

async fn admin(action: AdminAction) -> Result<()> {
    let database_url = match &action {
        AdminAction::Add { database_url, .. }
        | AdminAction::List { database_url }
        | AdminAction::Password { database_url, .. }
        | AdminAction::Role { database_url, .. }
        | AdminAction::Enable { database_url, .. }
        | AdminAction::Disable { database_url, .. } => database_url,
    };
    let pool = db::connect(database_url).await?;
    db::migrate(&pool).await?;
    match action {
        AdminAction::Add {
            username,
            role,
            password,
            ..
        } => {
            if !auth::valid_role(&role) {
                anyhow::bail!("role must be owner, admin, operator, viewer, or reseller");
            }
            let password_hash = auth::hash_password(&password)?;
            let created =
                db::repo::create_admin(&pool, &username, &password_hash, &role, 0, 0, 0, 0).await?;
            println!("admin added: {} ({})", created.username, created.role);
        }
        AdminAction::List { .. } => {
            for entry in db::repo::list_admins(&pool).await? {
                println!(
                    "{}\t{}\t{}",
                    if entry.enabled { "on" } else { "off" },
                    entry.role,
                    entry.username
                );
            }
        }
        AdminAction::Password {
            username, password, ..
        } => {
            let entry = db::repo::get_admin_by_username(&pool, &username)
                .await?
                .ok_or_else(|| anyhow::anyhow!("admin '{username}' not found"))?;
            let password_hash = auth::hash_password(&password)?;
            db::repo::update_admin(
                &pool,
                entry.id,
                None,
                None,
                Some(&password_hash),
                None,
                None,
                None,
                None,
            )
            .await?;
            db::repo::delete_admin_sessions(&pool, entry.id).await?;
            println!("password changed and sessions revoked: {}", entry.username);
        }
        AdminAction::Role { username, role, .. } => {
            if !auth::valid_role(&role) {
                anyhow::bail!("role must be owner, admin, operator, viewer, or reseller");
            }
            let entry = db::repo::get_admin_by_username(&pool, &username)
                .await?
                .ok_or_else(|| anyhow::anyhow!("admin '{username}' not found"))?;
            db::repo::update_admin(
                &pool,
                entry.id,
                Some(&role),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
            println!("role changed: {} -> {}", entry.username, role);
        }
        AdminAction::Enable { username, .. } => {
            let entry = db::repo::get_admin_by_username(&pool, &username)
                .await?
                .ok_or_else(|| anyhow::anyhow!("admin '{username}' not found"))?;
            db::repo::update_admin(
                &pool,
                entry.id,
                None,
                Some(true),
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
            println!("admin enabled: {}", entry.username);
        }
        AdminAction::Disable { username, .. } => {
            let entry = db::repo::get_admin_by_username(&pool, &username)
                .await?
                .ok_or_else(|| anyhow::anyhow!("admin '{username}' not found"))?;
            db::repo::update_admin(
                &pool,
                entry.id,
                None,
                Some(false),
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
            db::repo::delete_admin_sessions(&pool, entry.id).await?;
            println!("admin disabled: {}", entry.username);
        }
    }
    Ok(())
}

async fn domain(action: DomainAction) -> Result<()> {
    let database_url = match &action {
        DomainAction::Add { database_url, .. }
        | DomainAction::List { database_url }
        | DomainAction::Remove { database_url, .. } => database_url,
    };
    let pool = db::connect(database_url).await?;
    db::migrate(&pool).await?;

    match action {
        DomainAction::Add { target, path, .. } => {
            let target = panel::PanelTarget::parse(&target, path.as_deref())?;
            db::repo::add_panel_domain(&pool, &target.host, &target.base_path).await?;
            println!("panel added: {}", target.url());
            println!(
                "point DNS and your TLS reverse proxy at honey-master; Host + path are now allowed"
            );
        }
        DomainAction::List { .. } => {
            let domains = db::repo::list_panel_domains(&pool).await?;
            if domains.is_empty() {
                println!(
                    "no panel domains. add one with: honey-master domain add example.com/panel"
                );
            } else {
                for domain in domains {
                    let state = if domain.enabled { "on" } else { "off" };
                    println!("{state}\thttps://{}{}", domain.host, domain.base_path);
                }
            }
        }
        DomainAction::Remove { target, path, .. } => {
            let target = panel::PanelTarget::parse(&target, path.as_deref())?;
            if db::repo::remove_panel_domain(&pool, &target.host, &target.base_path).await? {
                println!("panel removed: {}", target.url());
            } else {
                println!("nothing to remove: {}", target.url());
            }
        }
    }
    Ok(())
}

#[cfg(feature = "dial-acceptor")]
async fn dial_accept(database_url: &str, certs_dir: PathBuf, listen: &str) -> Result<()> {
    require_secret_key()?;
    let pool = db::connect(database_url).await?;
    db::migrate(&pool).await?;
    let registry = std::sync::Arc::new(Registry::new(pool.clone()));
    tunnel::run(pool, registry, certs_dir, listen).await
}

#[cfg(not(feature = "dial-acceptor"))]
async fn dial_accept(_database_url: &str, _certs_dir: PathBuf, _listen: &str) -> Result<()> {
    anyhow::bail!("dial acceptor not built — rebuild with: cargo build --features dial-acceptor")
}

async fn ping(agent: &str, certs_dir: &std::path::Path) -> Result<()> {
    tracing::info!(agent = %agent, "dialing agent");
    let mut client = AgentClient::connect(agent, certs_dir, "honey-agent").await?;

    let who = client.whoru().await?;
    tracing::info!(
        node_id = %who.node_id, host = %who.hostname,
        agent_version = %who.agent_version, singbox = %who.singbox_version,
        os = %who.os, "who r u -> got node identity"
    );

    let rtt = client.ping().await?;
    println!(
        "connected to node '{}' ({}), rtt {} ms",
        who.node_id, who.os, rtt
    );
    Ok(())
}

async fn migrate(database_url: &str) -> Result<()> {
    let pool = db::connect(database_url).await?;
    db::migrate(&pool).await?;
    let n = db::repo::count_nodes(&pool).await?;
    tracing::info!(code = "M0201", nodes = n, "migrations applied");
    println!("migrations applied. nodes in db: {n}");
    Ok(())
}

async fn reencrypt(database_url: &str) -> Result<()> {
    if !secret::is_enabled() {
        anyhow::bail!("HONEY_SECRET_KEY is not set — nothing to encrypt with");
    }
    let pool = db::connect(database_url).await?;
    let (users, inbounds) = db::repo::reencrypt_secrets(&pool).await?;
    tracing::info!(
        code = "M1101",
        users,
        inbounds,
        "re-encrypted stored secrets"
    );
    println!("re-encrypted {users} user secret(s) and {inbounds} reality key(s)");
    Ok(())
}

async fn rekey(old_key: &str, new_key: &str, database_url: &str) -> Result<()> {
    if old_key.trim().is_empty() || new_key.trim().is_empty() {
        anyhow::bail!("set HONEY_SECRET_KEY_OLD (old) and HONEY_SECRET_KEY (new)");
    }
    if old_key.trim() == new_key.trim() {
        anyhow::bail!("old and new keys are identical — nothing to rotate");
    }
    let pool = db::connect(database_url).await?;
    let (users, inbounds, admins) = db::repo::rekey_secrets(&pool, old_key, new_key).await?;
    tracing::info!(
        code = "M1101",
        users,
        inbounds,
        admins,
        "rotated master key"
    );
    println!("rekeyed {users} user secret(s), {inbounds} reality key(s), {admins} totp secret(s)");
    println!("now restart honey-master with HONEY_SECRET_KEY set to the new key.");
    Ok(())
}

async fn push(node_name: &str, database_url: &str, certs_dir: &std::path::Path) -> Result<()> {
    require_secret_key()?;
    let pool = db::connect(database_url).await?;
    let node = db::repo::get_node_by_name(&pool, node_name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no node named '{node_name}'"))?;

    let reg = Registry::new(pool);
    reg.connect_serve(&node, certs_dir).await?;
    let status = reg.push(node.id).await?;

    println!(
        "pushed spec to '{}' -> state={:?} pid={} {}",
        node.name,
        status.state(),
        status.pid,
        status.message
    );
    Ok(())
}

async fn serve(
    database_url: &str,
    certs_dir: PathBuf,
    listen: &str,
    api_token: Option<String>,
    tls: TlsMode,
) -> Result<()> {
    require_secret_key()?;
    let pool = db::connect(database_url).await?;
    db::migrate(&pool).await?;

    let state = api::AppState {
        registry: std::sync::Arc::new(Registry::new(pool.clone())),
        pool,
        certs_dir,
        api_token,
        login_limiter: std::sync::Arc::new(ratelimit::LoginLimiter::new()),
        subscription_limiter: std::sync::Arc::new(ratelimit::SubscriptionLimiter::new()),
    };
    api_serve(state, listen, tls).await
}

/// Paths to a TLS cert/key pair for built-in https.
#[derive(Clone)]
#[cfg_attr(not(feature = "tls"), allow(dead_code))]
struct TlsPaths {
    cert: PathBuf,
    key: PathBuf,
}

/// Parameters for built-in ACME cert issuance.
#[cfg_attr(not(feature = "acme"), allow(dead_code))]
struct AcmeParams {
    domains: Vec<String>,
    email: String,
    cache: PathBuf,
    staging: bool,
}

/// How the api/panel is served.
enum TlsMode {
    None,
    Static(TlsPaths),
    Acme(AcmeParams),
}

fn build_tls_mode(
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    acme_domain: Vec<String>,
    acme_email: Option<String>,
    acme_cache: PathBuf,
    acme_staging: bool,
) -> Result<TlsMode> {
    if !acme_domain.is_empty() {
        if tls_cert.is_some() || tls_key.is_some() {
            anyhow::bail!("use either --acme-domain or --tls-cert/--tls-key, not both");
        }
        let email = acme_email
            .ok_or_else(|| anyhow::anyhow!("--acme-email is required with --acme-domain"))?;
        return Ok(TlsMode::Acme(AcmeParams {
            domains: acme_domain,
            email,
            cache: acme_cache,
            staging: acme_staging,
        }));
    }
    match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => Ok(TlsMode::Static(TlsPaths { cert, key })),
        (None, None) => Ok(TlsMode::None),
        _ => anyhow::bail!("--tls-cert and --tls-key must be set together"),
    }
}

async fn api_serve(state: api::AppState, listen: &str, mode: TlsMode) -> Result<()> {
    let api_token = state.api_token.clone();
    let has_admin = db::repo::count_enabled_admins(&state.pool).await? > 0;
    let app = api::router(state);

    match mode {
        TlsMode::None => {
            let listener = tokio::net::TcpListener::bind(listen).await?;
            let local_addr = listener.local_addr()?;
            ensure_safe_api_bind(local_addr, api_token.as_deref(), has_admin)?;
            tracing::info!(code = "M0102", api = %local_addr, "rest api up (http)");
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await?;
            Ok(())
        }
        TlsMode::Static(tls) => {
            let addr = parse_listen(listen)?;
            ensure_safe_api_bind(addr, api_token.as_deref(), has_admin)?;
            serve_https(addr, app, tls).await
        }
        TlsMode::Acme(params) => {
            let addr = parse_listen(listen)?;
            ensure_safe_api_bind(addr, api_token.as_deref(), has_admin)?;
            serve_acme(addr, app, params).await
        }
    }
}

fn parse_listen(listen: &str) -> Result<SocketAddr> {
    listen
        .parse()
        .map_err(|_| anyhow::anyhow!("--api-listen must be host:port for https, got {listen}"))
}

#[cfg(feature = "tls")]
async fn serve_https(addr: SocketAddr, app: Router, tls: TlsPaths) -> Result<()> {
    https::serve(addr, app, tls.cert, tls.key).await
}

#[cfg(not(feature = "tls"))]
async fn serve_https(_addr: SocketAddr, _app: Router, _tls: TlsPaths) -> Result<()> {
    anyhow::bail!("https not built — rebuild with: cargo build --features tls")
}

#[cfg(feature = "acme")]
async fn serve_acme(addr: SocketAddr, app: Router, params: AcmeParams) -> Result<()> {
    acme::serve(
        addr,
        app,
        params.domains,
        params.email,
        params.cache,
        params.staging,
    )
    .await
}

#[cfg(not(feature = "acme"))]
async fn serve_acme(_addr: SocketAddr, _app: Router, _params: AcmeParams) -> Result<()> {
    anyhow::bail!("acme not built — rebuild with: cargo build --features acme")
}

fn ensure_safe_api_bind(addr: SocketAddr, api_token: Option<&str>, has_admin: bool) -> Result<()> {
    if api_token.is_some_and(str::is_empty) {
        anyhow::bail!("HONEY_API_TOKEN must not be empty");
    }
    if api_token.is_none() && !has_admin && !addr.ip().is_loopback() {
        tracing::error!(code = "M0111", %addr, "refusing to expose an unauthenticated api");
        anyhow::bail!(
            "refusing to expose api on {addr} without an active admin; run honey-master admin add or bind to loopback"
        );
    }
    Ok(())
}

fn require_secret_key() -> Result<()> {
    if !secret::is_enabled() {
        tracing::error!(code = "M0112", "no HONEY_SECRET_KEY, can't touch secrets");
        anyhow::bail!("HONEY_SECRET_KEY is required; run 'honey-master keygen', store it securely, then set the environment variable");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthenticated_api_is_loopback_only() {
        let loopback: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let external: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        assert!(ensure_safe_api_bind(loopback, None, false).is_ok());
        assert!(ensure_safe_api_bind(external, None, false).is_err());
        assert!(ensure_safe_api_bind(external, None, true).is_ok());
        assert!(ensure_safe_api_bind(external, Some("token"), false).is_ok());
        assert!(ensure_safe_api_bind(loopback, Some(""), true).is_err());
    }
}
/// the long-lived service: rest api + reconcile loop (+ dial acceptor when built
/// with --features dial-acceptor), all sharing one pool and registry.
async fn run_service(
    database_url: &str,
    certs_dir: PathBuf,
    api_listen: String,
    dial_listen: String,
    api_token: Option<String>,
    reconcile_secs: u64,
    tls: TlsMode,
) -> Result<()> {
    let pool = db::connect(database_url).await?;
    require_secret_key()?;
    db::migrate(&pool).await?;

    let registry = std::sync::Arc::new(Registry::new(pool.clone()));
    let state = api::AppState {
        pool: pool.clone(),
        registry: registry.clone(),
        certs_dir: certs_dir.clone(),
        api_token,
        login_limiter: std::sync::Arc::new(ratelimit::LoginLimiter::new()),
        subscription_limiter: std::sync::Arc::new(ratelimit::SubscriptionLimiter::new()),
    };

    let mut set: tokio::task::JoinSet<Result<()>> = tokio::task::JoinSet::new();

    // HA leader election first: every instance serves the API, but the singleton
    // background loops below only act while this instance holds the lease.
    {
        let p = pool.clone();
        let ttl = std::env::var("HONEY_HA_LEASE_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(15)
            .clamp(5, 300);
        set.spawn(async move { ha::run(p, std::time::Duration::from_secs(ttl)).await });
    }
    {
        let listen = api_listen.clone();
        set.spawn(async move { api_serve(state, &listen, tls).await });
    }
    {
        let (p, r, c) = (pool.clone(), registry.clone(), certs_dir.clone());
        set.spawn(async move {
            reconcile::run(p, r, c, std::time::Duration::from_secs(reconcile_secs)).await
        });
    }
    {
        let (p, r) = (pool.clone(), registry.clone());
        set.spawn(async move { stats::run(p, r, std::time::Duration::from_secs(10), 1000).await });
    }
    {
        let p = pool.clone();
        set.spawn(
            async move { domains::monitor(p, std::time::Duration::from_secs(6 * 3600)).await },
        );
    }
    {
        let (p, r) = (pool.clone(), registry.clone());
        set.spawn(async move { quota::run(p, r, std::time::Duration::from_secs(300)).await });
    }
    {
        let (p, r) = (pool.clone(), registry.clone());
        set.spawn(async move { reach::monitor(p, r, std::time::Duration::from_secs(120)).await });
    }
    {
        let (p, r) = (pool.clone(), registry.clone());
        set.spawn(async move { schedule::run(p, r, std::time::Duration::from_secs(20)).await });
    }
    {
        let p = pool.clone();
        set.spawn(
            async move { stats::retention(p, std::time::Duration::from_secs(6 * 3600)).await },
        );
    }
    {
        let p = pool.clone();
        set.spawn(
            async move { notify::retention(p, std::time::Duration::from_secs(6 * 3600)).await },
        );
    }
    {
        let p = pool.clone();
        set.spawn(
            async move { monitor::anomaly_loop(p, std::time::Duration::from_secs(3600)).await },
        );
    }
    {
        let p = pool.clone();
        set.spawn(async move {
            monitor::status_sample_loop(p, std::time::Duration::from_secs(60)).await
        });
    }
    {
        let (p, r) = (pool.clone(), registry.clone());
        set.spawn(async move {
            monitor::device_limit_loop(p, r, std::time::Duration::from_secs(120)).await
        });
    }
    {
        let (p, r) = (pool.clone(), registry.clone());
        set.spawn(
            async move { monitor::drift_loop(p, r, std::time::Duration::from_secs(300)).await },
        );
    }
    if let Ok(token) = std::env::var("HONEY_TELEGRAM_TOKEN") {
        if !token.trim().is_empty() {
            let p = pool.clone();
            let public = std::env::var("HONEY_PUBLIC_URL").ok();
            set.spawn(async move { telegram::run(p, token, public).await });
        }
    }
    #[cfg(feature = "dial-acceptor")]
    {
        let (p, r, c, l) = (
            pool.clone(),
            registry.clone(),
            certs_dir.clone(),
            dial_listen.clone(),
        );
        set.spawn(async move { tunnel::run(p, r, c, &l).await });
    }
    #[cfg(not(feature = "dial-acceptor"))]
    {
        let _ = &dial_listen;
        tracing::info!(
            "dial acceptor not built (rebuild with --features dial-acceptor to accept NAT nodes)"
        );
    }

    tracing::info!(code = "M0101", api = %api_listen, "honey master running");

    // all tasks are infinite; the first to return means something broke.
    if let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(())) => tracing::warn!(code = "M0108", "a service task exited on its own"),
            Ok(Err(e)) => tracing::error!(code = "M0109", "service task failed: {e:#}"),
            Err(e) => tracing::error!(code = "M0110", "service task panicked: {e}"),
        }
    }
    set.shutdown().await;
    Ok(())
}
