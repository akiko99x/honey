<p align="center">
  <kbd>
    <img src="./d38ce393-e78d-4fbe-86d9-cb813fdaa5e1.png" width="100%" alt="honey logo">
  </kbd>
</p>

# honey

Universal, multi-node VPN panel. A **master** orchestrates many **nodes**; each
node runs **sing-box** (priority core) and **xray** at once, provisioned from one
panel.

- **master** — Rust. Panel, PostgreSQL, REST API, orchestration. gRPC *client* to
  the agents.
- **agent** — Go. Runs on every node, drives sing-box and xray directly via their
  native APIs, serves gRPC to the master.
- **link** — gRPC over mTLS. Two transports: the master dials the node (`serve`)
  or the node dials the master (`dial`, NAT-friendly).
- **stats** — per-user traffic from sing-box's Clash API and xray's StatsService,
  aggregated in the master for quota/expiry enforcement.

Every stored VPN credential is encrypted (uuid / password / REALITY key) or
hashed (admin password, sessions, subscription token) — no plaintext secrets in
the database.

## Requirements

- Linux node(s), root or `sudo` for install.
- Build host: **Rust** (stable) + **Go ≥ 1.23**. Only needed to produce the three
  binaries; nodes just run them.
- **PostgreSQL** reachable from the master.
- `sing-box` and `xray` binaries on each node. Pin the exact builds you validate
  (see [Versions](#versions)); honey shells out to them for `check` and `run`.
- A domain for the panel and a reverse proxy (or the built-in `tls`/`acme`
  features) for HTTPS.

## Build

Produces `honey-master`, `honey-agent` and `honey-enroll`:

```bash
mkdir -p dist
cargo build --release --manifest-path master/Cargo.toml --features dial-acceptor
cp master/target/release/honey-master dist/honey-master
(cd agent && go build -o ../dist/honey-agent ./cmd/agent)
(cd agent && go build -o ../dist/honey-enroll ./cmd/enroll)
```

Optional master features: `dial-acceptor` (accept NAT nodes that dial in), `tls`
(in-process HTTPS), `acme` (built-in ACME). They pull version-sensitive deps, so
they stay out of the default build.

## Install (single-server)

Master, agent and both cores co-host fine on one box. Bind the agent gRPC and all
core APIs to loopback; expose only the VPN inbound ports.

For a full single-server bootstrap, run this on a clean Ubuntu/Debian host:

```bash
curl -fsSL https://raw.githubusercontent.com/akiko99x/honey/main/scripts/bootstrap.sh \
  -o /tmp/honey-bootstrap.sh
sudo bash /tmp/honey-bootstrap.sh
rm -f /tmp/honey-bootstrap.sh
```

The bootstrap asks for the GitHub repository/release, panel domain/path and
owner credentials. It installs PostgreSQL, Caddy, the verified honey release,
the database, master certificates and systemd services. By default it also
installs SHA-256-verified sing-box/Xray releases and enrolls this server as the
first local VPN node.

For a local build or an unpacked source tree:

```bash
# 1. binaries + systemd units (see deploy/systemd/ for the unit files)
sudo ./scripts/install.sh dist/honey-master dist/honey-agent dist/honey-enroll

# 2. database + master key (store the key OUTSIDE the database — see Backup)
export DATABASE_URL=postgres://honey:honey@127.0.0.1:5432/honey
export HONEY_SECRET_KEY=$(honey-master keygen)
honey-master migrate

# 3. first admin
export HONEY_ADMIN_PASSWORD='replace-with-a-long-password'
honey-master admin add owner --role owner

# 4. allow the panel host (deny-by-default: exact Host + non-root path)
honey-master domain add panel.example.com/honey

# 5. run the master (API on loopback, dial acceptor for NAT nodes)
honey-master run --api-listen 127.0.0.1:8080 --dial-listen 0.0.0.0:9443
```

Put a reverse proxy in front for TLS (proxy the **whole** host — the panel, the
protected API and public `/sub/...` links share the origin):

```caddyfile
panel.example.com {
  reverse_proxy 127.0.0.1:8080
}
```

Open `https://panel.example.com/honey/` and sign in; the browser gets a 12-hour
HttpOnly session cookie. Domain add/remove is read from PostgreSQL per request —
no master restart needed.

To skip the proxy, build with `--features tls` and pass
`--tls-cert`/`--tls-key` (certs hot-reload hourly), or `--features acme` with
`--acme-domain`/`--acme-email` (TLS-ALPN-01, needs `:443` free — not the
single-server case where REALITY owns 443).

## First node → inbound → user → subscription

Do this in the panel, or over the API. Enroll the node with a one-time token so
its mTLS key never leaves the node:

```bash
# 1. create the node in the panel/API, then issue an enrollment token for it;
#    the API returns a ready install command:
#      sudo -u honey /opt/honey/bin/honey-enroll --master https://panel.example.com/honey --token <token> --listen 127.0.0.1:8443
#    run enrollment as the systemd honey user so its 0600 private key is readable by the service; the agent generates its key locally and receives a short-lived certificate.

# 2. run the agent (systemd unit does this; manual form shown for clarity):
honey-agent --mode serve --listen 127.0.0.1:8443 --node-id <node-uuid> \
  --ca /etc/honey/certs/ca.crt \
  --cert /etc/honey/certs/agent.crt --key /etc/honey/certs/agent.key
```

Then, in the panel: add an **inbound** on the node (e.g. VLESS+REALITY on 443 or
Hysteria2 on a UDP port), add a **user**, and `push`. Access is node-group based:
an ungrouped node is universal, while a grouped node is visible only to users
sharing one of its groups. New users receive the default group. The user's
subscription is served at:

- `GET /sub/:token` — styled status page (send `Accept: application/json` for data);
- `GET /sub/:token/v2ray` — base64 links + `Subscription-Userinfo` (quota/expiry);
- `GET /sub/:token/links` — the same links as plain text;
- `GET /sub/:token/sing-box` — a ready sing-box client config;
- `GET /sub/:token/sing-box-tun` — sing-box config with a system-wide TUN inbound;
- `GET /sub/:token/clash` — Clash/Mihomo config with auto-select and routing rules;
- `GET /sub/:token/qr/:inbound_id` — QR for one endpoint.

Disabled / expired / over-quota users still get a status page, but config and QR
downloads return `410 Gone`.

Quota/expiry is enforced automatically: when a user goes inactive the next
reconcile rebuilds the node spec without them; reset traffic or raise the limit
and they return on the next tick — no manual disable/enable.

## Ports

| Port                | Who            | Bind          | Notes                                   |
|---------------------|----------------|---------------|-----------------------------------------|
| API / panel `8080`  | master         | loopback      | put a reverse proxy in front for HTTPS  |
| `9443` (dial)       | master         | public        | only if you accept NAT nodes that dial  |
| agent gRPC `8443`   | agent          | loopback\*    | \*public only for a remote master in `serve` mode |
| Clash API / xray API| cores          | loopback      | stats plane; never expose               |
| VPN inbound ports   | cores          | public        | e.g. 443 (REALITY), your Hysteria2 UDP  |

Before deploying to a new IP, verify TCP `22`/`443` and the UDP inbound ports are
reachable from the target networks — a filtered IP is an infrastructure problem,
not a honey defect.

## DNS / TLS

- **Panel domain**: A/AAAA to the master; TLS via your reverse proxy or the
  `tls`/`acme` features.
- **Node domains**: point at each node; used for inbound `server_name` and CDN
  hostnames. sing-box TLS inbounds can use native ACME (HTTP-01 on port 80);
  Xray TLS inbounds use provisioned `cert_path`/`key_path` files.
- **REALITY `dest`/SNI** remain free text. A compatible public donor is the
  conventional choice, but filtering can be route-specific. A domain you own
  that resolves to the node and terminates a compatible TLS 1.3 target is also
  valid. Test the exact SNI from every target network. A no-SNI profile is an
  optional compatibility mode, not a universal or necessarily durable bypass.

## Backup & recovery

- **Database**: the installer includes `honey-backup.timer`. The backup script
  writes an atomic custom dump plus SHA-256, keeps 14 by default
  (`HONEY_BACKUP_KEEP`), and optionally encrypts with GPG
  (`HONEY_BACKUP_GPG_RECIPIENT`). Rehearse it with `restore-check.sh`.
- **Master key**: `HONEY_SECRET_KEY` encrypts uuid/password/REALITY keys. It is
  **not** in the database — back it up separately and securely. **Losing it makes
  those secrets unrecoverable.** A database restore without the matching key is
  useless.
- **Node restart**: the agent implements resume from an explicit active marker
  and a hash-verified last-applied config. Treat reboot/offline convergence as a
  prerelease acceptance gate until it has passed on the target Linux layout.
- **API smoke test**: `HONEY_API_TOKEN=... scripts/e2e-api.sh` exercises the
  control-plane lifecycle (node/inbound/user, enrollment issue+revoke, credential
  and subscription rotation, traffic reset, disable→410) against a disposable
  master. The real network path (client connect, counters, restart) is a separate
  Linux integration check.

## Versions

Pin the `sing-box` and `xray` builds you actually test, and record the clients
validated against each release. The `v0.0.0` release smoke used sing-box
**1.13.14** and Xray **26.3.27** on Ubuntu 24.04; VLESS+REALITY and Hysteria2
both passed from a Windows client. This is release evidence, not a generic
compatibility promise for every client or network.

## Protocols & transports

- **Protocols**: VLESS (+REALITY), VMess, Hysteria2, Trojan, TUIC, Shadowsocks,
  AnyTLS, ShadowTLS.
- **Transports**: both cores support tcp/ws/grpc/http/h2/httpupgrade/quic;
  xhttp and mkcp are Xray-only. uTLS fingerprint and ECH hints are modelled.
  VLESS+WS behind a CDN is emitted in v2ray, Clash and sing-box formats. Xray
  xHTTP is emitted as a v2ray-style link; unsupported Clash/sing-box client
  outputs omit it instead of silently degrading it to raw TCP.

## Troubleshooting

- **`push` fails / node never comes online** — the master must reach the agent
  (`serve`: master → node `:8443`; `dial`: node → master `:9443`), the agent's
  `--node-id` must equal the node's database id, and mTLS certs must match the CA.
  Check the agent logs and the node's push history in the panel.
- **Core won't start after an edit** — every config is validated with the core's
  own `check` before it replaces the running one; a failed apply restores the
  previous config. Read the core error surfaced on the inbound / in the logs.
- **Client can't connect but the node is "online"** — "agent online" (control
  plane) is not the same as "endpoint reachable". Test the VPN port itself from
  the target network; an IP/route filter looks like a total timeout while the
  panel still shows the node up.
- **Master refuses to start** — runtime commands require `HONEY_SECRET_KEY`, and
  the master will not bind a non-loopback API without an enabled admin (or a
  legacy `HONEY_API_TOKEN`).

## Layout

```
honey/
├── proto/                 # shared gRPC contract (buf) — single source of truth
├── master/                # Rust: panel, REST API, db, orchestration, subscriptions
│   ├── migrations/        # PostgreSQL schema (sqlx)
│   └── src/               # api/, db/, spec.rs, registry.rs, reconcile.rs, stats.rs, ...
├── agent/                 # Go: gRPC server on nodes
│   ├── cmd/agent/         # agent entrypoint (+ restart recovery)
│   ├── cmd/enroll/        # honey-enroll one-time enrollment client
│   └── internal/          # core/, singbox/, xray/, mtls/, transport/, grpcserver/
├── web/                   # embedded panel + subscription page (served by the master)
├── deploy/systemd/        # master, agent and backup service/timer units
├── scripts/               # install, release, migration, backup/restore and e2e tools
└── docs/                  # handbook, focused guides and runbooks
```

Start with the [operator and developer handbook](docs/handbook.md). It is the
current end-to-end guide for installation, lifecycle, API, operations and
release work. See [docs/transports.md](docs/transports.md) for the transport model and
[docs/example-node.md](docs/example-node.md) for a worked node spec. Operational
health is documented in [docs/issues.md](docs/issues.md), operational
organization in [docs/labels-saved-views.md](docs/labels-saved-views.md),
session security in [docs/admin-sessions.md](docs/admin-sessions.md), recovery
codes in [docs/admin-recovery.md](docs/admin-recovery.md), and the notification
bell in [docs/notifications.md](docs/notifications.md). The DB-derived first-
run checklist, reseller scope and auth UX are documented in
[docs/onboarding.md](docs/onboarding.md). Public subscription rate limits,
privacy guarantees and abuse visibility are covered in
[docs/subscription-guard.md](docs/subscription-guard.md); bounded runtime
diagnostics and request-ID correlation are documented in
[docs/log-search.md](docs/log-search.md). Named bearer keys and the integration
surface are documented in [docs/api-keys.md](docs/api-keys.md); the stable
automation contract is [web/openapi.json](web/openapi.json), served at
`/openapi.json`, while the handbook catalogs the additional panel routes.
Before a tag, follow [docs/release-checklist.md](docs/release-checklist.md).
Repository owners should also follow the
[GitHub publication guide](docs/github-publication.md) for the initial push,
repository security settings and first automated release.

## License

Honey's own source code is licensed under the
[GNU Affero General Public License v3.0 only](LICENSE) (`AGPL-3.0-only`). There
is no user-count limit or separate commercial license for the initial release.
The AGPL permits commercial use, but modified versions offered to users over a
network must provide those users access to the corresponding source under the
same license.

Xray, sing-box, and other third-party components retain their own licenses and
are not relicensed as part of Honey. A separate commercial license may be
offered in the future, without changing the terms already granted for released
AGPL versions.


See docs/traffic-analytics.md for the historical usage API and disposable smoke.
