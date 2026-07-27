# honey handbook

This is the current operator and developer guide for honey. It is intentionally
one document: the smaller files in `docs/` remain focused runbooks and design
notes, while this page is the starting point for a fresh installation, an API
integration, or a release review.

The product keeps its loose English UI and log messages, but the operational
rules are strict: the control plane is authenticated, VPN credentials are
encrypted at rest, node links use mTLS, and a new core configuration is checked
before it can replace a running process.

## 1. What honey is

Honey is a Master–Agent VPN control plane.

| Component | Runtime | Responsibility |
|---|---|---|
| `honey-master` | Rust + PostgreSQL | panel, REST API, database, orchestration, subscriptions, quotas, alerts |
| `honey-agent` | Go | node gRPC server, mTLS, core lifecycle, config validation, local accounting |
| `honey-enroll` | Go | one-time node enrollment, local key generation and certificate install |
| sing-box | external binary | priority data-plane core, including Hysteria2, TUIC, AnyTLS and Shadowsocks |
| Xray | external binary | VLESS/VMess/Trojan/Reality and Xray-specific transports |
| Caddy or another proxy | optional | public HTTPS for the panel, API and subscription origin |

The master never receives an agent private key during enrollment. The agent
creates its key locally, sends a CSR, and receives a short-lived certificate.
The database stores hashes for admin/session/subscription bearer material and
encrypted values for VPN credentials such as UUIDs, passwords and REALITY
private keys.

## 2. Supported topologies

### Single-server

The master, agent, sing-box and Xray run on one Linux host:

```text
browser/client ── HTTPS ──> reverse proxy ──> master:127.0.0.1:8080
                                      └────> /sub/* and API
master ── mTLS/gRPC ──> agent:127.0.0.1:8443
agent ──> sing-box/Xray ── public VPN ports
```

This is the fastest smoke-test and personal deployment path. Do not expose the
Clash API, Xray stats API, PostgreSQL, or the master HTTP listener directly.

### Remote serve node

The master dials the node at its `address:grpc_port`. The node must be reachable
from the master and its certificate SAN must match the node's `tls_server_name`.

### NAT/dial node

Build the master with `dial-acceptor` and expose the master's dial listener:

```text
node ── mTLS/gRPC outbound ──> master:9443
master ──> existing tunnel ── agent RPCs
```

Use `HONEY_MODE=dial` or `both` on the agent. The dial listener is not the panel
HTTP listener and does not replace the reverse proxy.

## 3. Fast install on Ubuntu/Debian

Docker Compose is the preferred clean-server deployment beginning with v0.0.6.
The installer installs Docker Engine/Compose when needed, pulls tagged honey
images, creates persistent volumes and root-only secrets, starts PostgreSQL,
Caddy and backups, and can enroll the host as the first local serve node.

```bash
curl -fsSL https://raw.githubusercontent.com/akiko99x/honey/main/scripts/install-docker.sh \
  -o /tmp/honey-install-docker.sh
sudo bash /tmp/honey-install-docker.sh
rm -f /tmp/honey-install-docker.sh
```

Interactive prompts cover:

1. panel domain;
2. owner username and password;
3. whether this host should become the first VPN node.

The DNS A/AAAA record must already point to the host. TCP `80`/`443` are needed
for Caddy certificate issuance and HTTPS. If the host uses TCP `443` for
REALITY, keep the panel on a separate proxy/port arrangement; Hysteria2 uses
UDP and can coexist on a different UDP port.

For automation:

```bash
sudo env \
  HONEY_PANEL_DOMAIN=panel.example.com \
  HONEY_ADMIN_USERNAME=owner \
  HONEY_ADMIN_PASSWORD='use-a-long-secret' \
  HONEY_INSTALL_LOCAL_NODE=1 \
  bash /tmp/honey-install-docker.sh --non-interactive
```

The installer refuses to run beside an active systemd honey deployment because
the two stacks share host ports. Rehearse Docker on a clean server and follow
[`docker-deployment.md`](docker-deployment.md) for upgrades, health checks and
backup/restore rehearsals.

## 4. Legacy systemd and manual installation

The previous `scripts/bootstrap.sh`, release installer and systemd units remain
available as a compatibility and recovery path.

`scripts/install-release.sh` downloads a published Linux release and verifies
the archive SHA-256 before invoking the safe installer:

```bash
sudo bash scripts/install-release.sh \
  --repo akiko99x/honey \
  --version v0.0.0 \
  --enable --start
```

For an archive already on disk:

```bash
sudo bash scripts/install.sh --start --enable \
  honey-0.0.0-linux-amd64.tar.gz
```

For three locally-built binaries:

```bash
sudo bash scripts/install.sh --start --enable \
  dist/honey-master dist/honey-agent dist/honey-enroll
```

The installer:

- creates the locked `honey` service account;
- installs binaries under `/opt/honey/bin`;
- installs systemd units and backup timer;
- preserves existing `/etc/honey` configuration;
- creates required state directories;
- does not start services unless `--start` is supplied.

`--allow-unverified` is only for a trusted local archive without a checksum. Do
not use it for a network download.

## 5. Build from source

Requirements: stable Rust, Go matching `agent/go.mod` (Go 1.25 or newer), Node
for syntax checks, PostgreSQL for integration/lifecycle scripts, and Buf when
regenerating protobuf stubs.

```bash
cargo fmt --manifest-path master/Cargo.toml
cargo test --locked --manifest-path master/Cargo.toml
cargo test --locked --manifest-path master/Cargo.toml --features dial-acceptor

(cd agent && go test ./...)
node --check web/app.js
node --check web/subscription.js
bash -n scripts/*.sh
```

Build the three Linux binaries:

```bash
cargo build --locked --release \
  --manifest-path master/Cargo.toml \
  --features dial-acceptor,acme

 (cd agent && CGO_ENABLED=0 GOOS=linux GOARCH=amd64 \
  go build -trimpath -ldflags='-s -w' -o ../dist/honey-agent ./cmd/agent)
 (cd agent && CGO_ENABLED=0 GOOS=linux GOARCH=amd64 \
  go build -trimpath -ldflags='-s -w' -o ../dist/honey-enroll ./cmd/enroll)
cp master/target/release/honey-master dist/honey-master
```

Use `scripts/package-release.sh` to create the checksummed archive. Use
`scripts/release-readiness.sh --static`, `--full`, or `--package` as the release
gate. Package mode must run on Linux.

## 6. Runtime configuration

The example files are the source for systemd environment names:

- `deploy/systemd/master.env.example`
- `deploy/systemd/agent.env.example`

### Master

| Variable | Meaning |
|---|---|
| `DATABASE_URL` | PostgreSQL connection string |
| `HONEY_SECRET_KEY` | base64 key for encrypted VPN secrets; keep outside the database |
| `HONEY_SECRET_KEY_FILE` | alternative secret-key file backend |
| `HONEY_SECRET_KEY_COMMAND` | command backend for the key |
| `HONEY_VAULT_ADDR` and related variables | HashiCorp Vault KV backend |
| `HONEY_CERTS_DIR` | master/CA certificate directory |
| `HONEY_API_LISTEN` | REST/panel bind, normally `127.0.0.1:8080` |
| `HONEY_DIAL_LISTEN` | NAT-node acceptor, normally `0.0.0.0:9443` |
| `HONEY_API_TOKEN` | legacy machine bearer token; prefer named API keys |
| `HONEY_TLS_CERT`, `HONEY_TLS_KEY` | built-in HTTPS certificate/key |
| `HONEY_ACME_EMAIL`, `HONEY_ACME_CACHE` | built-in ACME settings |
| `HONEY_UPDATE_REPO` | GitHub repository used by software update |
| `HONEY_UPDATE_AUTO_RESTART` | systemd-only opt-in for staged self-update restart |
| `HONEY_LOG_FORMAT` | `pretty` or structured `json` logging |
| `HONEY_GEOIP_FILE` | optional GeoIP country table |
| `HONEY_HA_LEASE_SECS` | HA lease duration |

Generate a new key with:

```bash
honey-master keygen
```

Never regenerate or replace the key without a tested database backup and a
planned `rekey` operation. Losing the active key makes encrypted VPN material
unrecoverable.

### Agent

| Variable | Meaning |
|---|---|
| `HONEY_MODE` | `serve`, `dial`, or `both` |
| `HONEY_LISTEN` | agent gRPC listener for serve/both |
| `HONEY_MASTER_ADDR` | master dial acceptor for dial/both |
| `HONEY_NODE_ID` | UUID matching the node row in the master |
| `HONEY_SINGBOX_BIN` | sing-box executable |
| `HONEY_XRAY_BIN` | Xray executable |
| `HONEY_CLASH_URL` | loopback sing-box Clash API |
| `HONEY_CLASH_SECRET` | optional Clash API secret |

Core APIs and PostgreSQL should remain loopback-only.

## 7. First-run lifecycle

### 7.1 Master initialization

```bash
sudo -u honey bash -lc '
  set -a
  . /etc/honey/master.env
  set +a
  /opt/honey/bin/honey-master migrate
  export HONEY_ADMIN_PASSWORD="replace-with-a-long-password"
  /opt/honey/bin/honey-master admin add owner --role owner
'
```

Allow the exact panel host/path before opening the browser:

```bash
sudo -u honey bash -lc '
  set -a; . /etc/honey/master.env; set +a
  /opt/honey/bin/honey-master domain add panel.example.com/panel
'
```

### 7.2 Node enrollment

Create a node in the panel or through `POST /nodes`, then issue its one-time
enrollment token. Run the generated command on the node as the service account:

```bash
sudo -u honey /opt/honey/bin/honey-enroll \
  --master https://panel.example.com/panel \
  --token '<one-time-token>' \
  --listen 0.0.0.0:8443
```

The token is single-use. The private key is generated on the node. The master
must be able to validate the resulting certificate chain and SAN.

### 7.3 Inbound and user lifecycle

1. Create or select a node.
2. Create an inbound with a unique tag and port.
3. Configure its protocol/core/security and certificate paths.
4. Create a user and set quota/expiry/device limits.
5. Push the desired state, or wait for enabled auto-push/reconcile.
6. Import the user's subscription into a client.

The agent builds one config per core, validates candidates with the core's own
checker, and swaps the running process only after validation. A failed apply
keeps the previous working config where rollback is possible.

## 8. Protocols, cores and ports

The master validates core/protocol/transport compatibility before the agent sees
the spec.

| Use case | Preferred core | Typical transport |
|---|---|---|
| VLESS + REALITY | Xray | TCP/RAW |
| VLESS + CDN | Xray or sing-box | WebSocket/gRPC/xHTTP as supported |
| Hysteria2 | sing-box | UDP/QUIC, optional port hopping |
| TUIC | sing-box | UDP/QUIC |
| AnyTLS / Shadowsocks | sing-box | protocol-specific |
| VMess / Trojan | Xray or sing-box | protocol-specific |

Common single-server binds:

| Port | Service | Exposure |
|---|---|---|
| `127.0.0.1:8080` | master REST/panel | reverse proxy only |
| `0.0.0.0:9443` | master dial acceptor | public only for NAT nodes |
| `:8443` | agent gRPC | public only for remote serve nodes |
| `127.0.0.1:9090` | sing-box Clash API | never public |
| `127.0.0.1:8081` | Xray stats/API | never public |
| `127.0.0.1:9080` | Honey Xray ACME HTTP-01 gateway | Caddy challenge upstream only |
| inbound ports | sing-box/Xray | public VPN traffic |

### Hysteria2 checklist

- Use a UDP port not already owned by another service.
- Use TLS with a certificate whose SAN matches the configured server name.
- Copy the certificate/key into a honey-readable directory, for example
  `/etc/honey/sing-box/tls`.
- For UDP port hopping, set the hop range and ensure the agent can manage nftables.
- Confirm `sing-box check -c /etc/honey/sing-box/config.json`.
- Verify that the process is listening with `ss -lunp`.

### REALITY checklist

- The client public key, short ID, SNI and target must match the server config.
- A foreign SNI that does not resolve or route correctly can be blocked by an
  upstream filter even when the keypair is correct.
- A domain you control is often easier to troubleshoot because its DNS and
  certificate path are observable.
- Test the exact client network; “node online” only proves the control plane.

## 9. Subscriptions and client output

The public subscription origin is served by the master and normally sits behind
the same HTTPS host as the panel:

| Route | Output |
|---|---|
| `GET /sub/:token` | styled status page or JSON with `Accept: application/json` |
| `GET /sub/:token/v2ray` | base64 links and `Subscription-Userinfo` |
| `GET /sub/:token/links` | one client link per line |
| `GET /sub/:token/sing-box` | sing-box client JSON |
| `GET /sub/:token/sing-box-tun` | sing-box JSON with a system-wide TUN inbound |
| `GET /sub/:token/clash` | Clash/Mihomo YAML with routing and auto-select |
| `GET /sub/:token/qr` | page/collection of endpoint QR codes |
| `GET /sub/:token/qr/:inbound_id` | QR for one endpoint |
| `GET /sub/:token/speedtest` | bounded client-to-panel download test |
| `GET /sub/:token/services` | managed MTProto/Naive service links |
| `GET /sub/:token/wireguard` | available WireGuard interfaces |
| `GET /sub/:token/wireguard/:iface_id` | WireGuard client configuration |
| `GET /sub/:token/wireguard/:iface_id/qr` | WireGuard configuration QR |
| `GET /s/:alias` | status page through a configured subscription alias |

Disabled, expired and over-quota users retain the status page but receive
`410 Gone` for configuration and QR downloads. Public subscription responses
use private/no-store headers and rate limiting; do not paste subscription URLs
into logs or issue trackers.

## 10. REST API

The machine-readable stable automation contract is embedded at `/openapi.json`
and is also stored in `web/openapi.json`. It does not yet describe every
panel-only route; the complete route catalog is included below. The API uses
JSON and UUID resource IDs.
Send `Authorization: Bearer <hny_...>` for a named API key or use the browser's
HttpOnly session cookie. `health` and `ready` are unauthenticated; all control
routes require an authenticated identity.

### System and identity

| Method | Route | Purpose |
|---|---|---|
| GET | `/health` | process liveness |
| GET | `/ready` | database readiness |
| GET | `/openapi.json` | machine-readable API contract |
| GET | `/auth/me` | current account/API-key identity |
| GET | `/onboarding` | derived first-run progress |
| GET | `/issues` | fleet health cockpit |
| GET | `/system/logs` | bounded master log search |

### API keys

| Method | Route | Purpose |
|---|---|---|
| GET | `/api-keys` | list keys without plaintext tokens |
| POST | `/api-keys` | create a named scoped key; save the token once |
| DELETE | `/api-keys/{id}` | revoke a key immediately |

### Nodes

| Method | Route | Purpose |
|---|---|---|
| GET | `/nodes` | list nodes |
| POST | `/nodes` | create a node |
| GET | `/nodes/{id}` | get node details |
| PATCH | `/nodes/{id}` | edit address, TLS, labels and lifecycle fields |
| DELETE | `/nodes/{id}` | delete a node |
| POST | `/nodes/{id}/push` | manually push desired state |
| POST | `/nodes/{id}/dry-run` | validate a candidate on the agent |
| GET | `/nodes/{id}/config-preview` | sanitized desired-config diff |
| GET | `/nodes/{id}/inbounds` | list node inbounds |

Enrollment token issuance, certificate inventory/revocation and node logs are
available from the panel's node-scoped actions; use the OpenAPI document for
the exact request schemas when automating those operations.

### Inbounds

| Method | Route | Purpose |
|---|---|---|
| POST | `/inbounds` | create an inbound |
| GET | `/inbounds/{id}` | get an inbound |
| PATCH | `/inbounds/{id}` | update an inbound |
| DELETE | `/inbounds/{id}` | delete an inbound |

### Users and groups

| Method | Route | Purpose |
|---|---|---|
| GET | `/users` | list users visible to the identity |
| POST | `/users` | create a user and subscription credential |
| GET | `/users/{id}` | get a user |
| PATCH | `/users/{id}` | edit quota, expiry, device limit, labels and enabled state |
| DELETE | `/users/{id}` | delete a user |
| POST | `/users/{id}/reset-traffic` | reset traffic counters |
| GET | `/users/{id}/subscription` | reveal permanent and optional revocable links |
| GET | `/users/{id}/subscription-preview` | preview effective client metadata and compatibility profiles |
| POST | `/users/{id}/rotate` | optionally rotate protocol UUID/password credentials |
| POST | `/users/{id}/rotate-sub` | optionally rotate the revocable subscription token |
| GET | `/groups` | list groups |
| POST | `/groups` | create a group |

### Domains and routing

| Method | Route | Purpose |
|---|---|---|
| GET | `/domains` | list managed panel/endpoint domains |
| POST | `/domains` | register a domain |
| POST | `/domains/{id}/verify` | verify DNS, reachability and certificate |
| GET | `/routing-profiles` | list routing profiles |
| POST | `/routing-profiles` | create a routing profile |

### Traffic analytics

| Method | Route | Purpose |
|---|---|---|
| GET | `/analytics/traffic` | bounded historical traffic series |
| GET | `/analytics/traffic.csv` | CSV export of the same series |

For routes present there, treat `web/openapi.json` as authoritative for exact
payloads, validation bounds, role requirements, filters and response schemas.

### Extended panel/control routes

The embedded OpenAPI file currently describes the stable automation subset. The
panel uses additional authenticated routes below; their names are stable, but
the exact JSON shapes should be read from the corresponding panel request and
the Rust handler before writing an integration.

| Method | Route | Purpose |
|---|---|---|
| GET | `/announcement`, `/branding`, `/status` | public panel/subscription metadata |
| POST | `/auth/login` | create a browser session |
| POST | `/auth/logout` | end the current session |
| GET | `/auth/sessions`, `/auth/login-history` | list sessions and login events |
| POST | `/auth/sessions/revoke-others` | revoke every other session |
| DELETE | `/auth/sessions/{id}` | revoke one session |
| GET/POST | `/admins` | list or create administrators |
| PATCH | `/admins/{id}` | update an administrator |
| GET | `/admins/{id}/groups` | reseller group scope |
| GET/POST/PATCH/DELETE | `/custom-roles`, `/custom-roles/{id}` | custom RBAC matrices |
| POST | `/import/users` | import users from a controlled payload |
| GET/POST | `/config/export`, `/config/apply` | export or apply panel configuration |
| GET/POST | `/auth/totp`, `/auth/totp/setup`, `/auth/totp/enable` | TOTP status and setup |
| POST | `/auth/totp/disable`, `/auth/totp/recovery/generate` | disable TOTP or rotate recovery codes |
| GET | `/auth/totp/recovery` | recovery-code status |
| GET/POST/DELETE | `/admin-ips`, `/admin-ips/{id}` | administrator IP allowlist |
| PUT | `/users/{id}/quota-interval` | configure rolling quota interval |
| GET | `/audit`, `/audit/verify` | audit events and hash-chain verification |
| GET | `/reports/period`, `/analytics/geo` | period report and geography aggregation |
| GET | `/ha` | HA lease/leader status |
| GET/POST | `/update`, `/update/apply` | check and explicitly stage a master update |
| GET/PATCH | `/settings` | runtime settings, including auto-push |
| GET | `/notifications`, `/notifications/unread-count` | notification center |
| POST | `/notifications/read-all`, `/notifications/{id}/read` | mark notifications read |
| GET | `/labels`, `/saved-views` | label catalog and saved table views |
| POST/PATCH/DELETE | `/saved-views`, `/saved-views/{id}` | manage saved views |
| GET | `/metrics`, `/live-connections` | master metrics and active connections |
| GET/PATCH/DELETE | `/domains/{id}` | inspect or edit one managed domain |
| GET/PATCH/DELETE | `/routing-profiles/{id}` | edit or delete a routing profile |
| PUT | `/users/{id}/routing-profile` | assign a routing profile |
| GET/POST/PATCH/DELETE | `/notify-channels`, `/notify-channels/{id}` | notification channel management |
| POST | `/notify-channels/{id}/test` | send a channel test |
| GET/POST/DELETE | `/telegram-chats`, `/telegram-chats/{chat_id}` | Telegram chat registration |
| PUT | `/nodes/{id}/labels` | replace node labels |
| GET | `/nodes/{id}/config-drift`, `/nodes/{id}/preflight` | drift and reachability gates |
| POST | `/nodes/{id}/benchmark` | bounded control-channel benchmark |
| GET | `/nodes/{id}/pushes`, `/nodes/{id}/logs`, `/nodes/{id}/metrics` | node history/logs/metrics |
| GET/POST | `/nodes/{id}/enrollments` | list or issue enrollment claims |
| GET | `/nodes/{id}/certificates`, `/nodes/{id}/history` | certificate and change history |
| POST | `/nodes/{id}/revert/{version}` | revert a node version |
| POST | `/certificates/{id}/revoke`, `/enrollments/{id}/revoke` | revoke certificate or enrollment |
| PATCH | `/branding` | update white-label panel branding |
| GET/POST/PATCH/DELETE | `/announcements`, `/announcements/{id}` | operator announcement banner |
| GET/POST/DELETE | `/scheduled-ops`, `/scheduled-ops/{id}` | scheduled operations |
| POST | `/reality/keygen` | generate REALITY material for an inbound |
| GET/POST | `/nodes/{id}/wireguard` | list or create WireGuard interfaces |
| PATCH/DELETE | `/wireguard/{id}` | update or delete a WireGuard interface |
| GET/POST | `/nodes/{id}/services` | list or create managed external services |
| PATCH/DELETE | `/services/{id}` | update or delete a managed service |
| PUT | `/inbounds/{id}/labels` | replace inbound labels |
| GET | `/inbounds/{id}/history` | inbound change history |
| POST | `/inbounds/{id}/revert/{version}` | revert an inbound version |
| GET | `/users/{id}/history` | user change history |
| POST | `/users/{id}/revert/{version}` | revert a user version |
| PATCH/DELETE | `/groups/{id}` | edit or delete a group |
| GET/PUT | `/nodes/{id}/groups`, `/users/{id}/groups` | read or replace group membership |
| POST | `/inbounds/{id}/reach` | trigger an endpoint probe |
| GET/PUT | `/inbounds/{id}/reachability` | read/report vantage reachability |
| POST | `/inbounds/{id}/rotate-sni` | rotate a blocked REALITY/CDN SNI |
| PUT | `/users/{id}/labels`, `/users/{id}/alias` | replace labels or set subscription alias |
| POST | `/users/{id}/rotate-sub` | rotate the main subscription token |
| GET | `/users/{id}/subscription` | reveal the main subscription link for an authorized operator |
| GET/POST | `/users/{id}/subscriptions` | list or create named subscription links |
| GET/DELETE | `/users/{id}/subscriptions/{sid}` | reveal or delete a named subscription |
| GET/POST | `/users/{id}/gdpr-export`, `/users/{id}/gdpr-erase` | export or erase user data |

The public enrollment claim route is `POST /enroll/:token/claim`; the agent
enrollment client uses it once and then switches to mTLS. Public subscription
routes are listed in the next section and are protected by the subscription
guard rather than the admin session middleware.

`/sub-assets/subscription.css`, `/sub-assets/subscription.js` and
`/sub-assets/PretendardVariable.woff2` are static resources for the public
subscription page, not JSON API endpoints.

## 11. Authentication and permissions

Roles are `owner`, `admin`, `operator`, `viewer`, and `reseller`. A named API key
inherits the creator's role and can be narrowed with domain scopes and expiry.
Plaintext API tokens are returned only at creation.

Browser authentication uses an HttpOnly session cookie. Login throttling,
CSRF/origin checks, IP allowlists, TOTP and recovery codes are enforced by the
master. Resellers see only their own users/groups/subscriptions.

Never use a subscription token as an API token. Never put `HONEY_SECRET_KEY`,
private keys, enrollment bundles, database dumps or raw subscription URLs in
support tickets.

## 12. Auto-push, reconcile and self-update

Auto-push is controlled in Settings → Automation and by the persisted runtime
setting `auto_push_enabled`. When disabled, the master keeps node heartbeats,
stats and health monitoring but does not automatically apply desired configs.
Manual `POST /nodes/{id}/push` remains the explicit operator action.

Reconcile runs periodically and repairs drift when auto-push is enabled. A
deferred push after a panel mutation protects the HTTP response from a core
restart racing the browser request.

The Software settings page can check GitHub releases and stage a verified master
binary. The update path requires a release checksum, stages the binary beside
the running executable, and relies on systemd `Restart=always` when
`HONEY_UPDATE_AUTO_RESTART=1` is enabled. It never silently downloads or swaps
code without the operator action.

## 13. Backup, restore and disaster recovery

Create a database backup:

```bash
sudo bash scripts/backup-postgres.sh /var/lib/honey/backups
```

The script writes an atomic custom-format dump and `.sha256` file, keeps the
newest `HONEY_BACKUP_KEEP` artifacts, and can encrypt with GPG using
`HONEY_BACKUP_GPG_RECIPIENT`.

Verify checksum and rehearse a scratch restore:

```bash
cd /var/lib/honey/backups
sha256sum -c honey-<stamp>.dump.sha256
sudo -u postgres env ADMIN_DATABASE_URL=postgres:///postgres \
  bash /home/user/honey/scripts/restore-check.sh honey-<stamp>.dump
```

Back up separately:

1. the active secret-key backend or key material;
2. `/etc/honey/master-certs` and the node certificate authority;
3. `/etc/honey` service configuration;
4. Caddy certificates/configuration when using a reverse proxy.

After a restore, run migrations, start the master, verify `/health` and
`/ready`, verify node certificate inventory, and perform a controlled push.

## 14. Observability and troubleshooting

### First triage

```bash
systemctl status honey-master honey-agent --no-pager
journalctl -u honey-master -u honey-agent -n 200 --no-pager
curl -fsS http://127.0.0.1:8080/health
curl -fsS http://127.0.0.1:8080/ready
ss -ltnup
```

Use the panel's Issues, Logs, node Activity and Push history views. Search by
diagnostic code and request ID rather than copying an entire journal.

### Node offline

Check the direction first:

- `serve`: master → node `address:grpc_port`;
- `dial`: node → master `HONEY_DIAL_LISTEN`;
- `both`: either path may register the node.

Then verify node ID, certificate SAN, CA, certificate expiry/revocation, firewall
rules, and the actual listener. “Online” means the authenticated control
channel works; it says nothing about VPN endpoint reachability.

### Push or core failure

Look for `M0406`, `N0102`, `N0103`, `N0106`, `N0112` or `N0113`. Confirm the
candidate with a dry-run and run the core's own checker against the active
config. A failed validation should leave the previous working process intact.

### Client timeout

Test the exact public TCP/UDP port from the target network, then inspect
`M1501`/`M1502` reachability events. A filtered IP, UDP path or REALITY
ClientHello can fail outside honey while the node remains healthy.

## 15. Error and log codes

See [error-codes.md](error-codes.md) for the complete catalog. The short rule:

- `A####`: agent transport, enrollment and RPC lifecycle;
- `N####`: sing-box/Xray/core lifecycle and node-local services;
- `M####`: master runtime, API, registry, subscriptions and monitors.

API failures return a stable JSON object:

```json
{"error":"human-readable safe message","code":"M1201"}
```

The detailed upstream/database error belongs in the authenticated master log,
not in the public response.

## 16. Development and release checklist

Before a release:

1. add forward-only SQLx migrations; never edit a released migration;
2. update both master validation and agent builders for protocol changes;
3. regenerate Go protobuf files after editing `proto/`;
4. run Go tests, Rust tests, Rust feature checks, JS and shell syntax checks;
5. run `scripts/release-readiness.sh --full --allow-no-git` before GitHub setup;
6. run package mode on Linux and inspect archive contents;
7. verify checksums and test install/upgrade/rollback on a disposable host;
8. update this handbook, the focused runbook and `docs/error-codes.md`;
9. perform the live lifecycle smoke: enrollment, push, client traffic,
   accounting, quota/expiry cutoff, restart recovery and backup restore.

The repository is AGPL-3.0-only. Third-party sing-box, Xray, Caddy and library
licenses remain their own.

## 17. Documentation map

| Topic | Focused document |
|---|---|
| transports and protocol model | `docs/transports.md` |
| installation/deployment | `docs/deployment-runbook.md`, `docs/onboarding.md` |
| upgrades and self-update | `docs/upgrades.md` |
| backups and operations | `docs/runbooks.md`, `docs/secrets.md` |
| auto-push | `docs/auto-push.md` |
| errors and diagnostics | `docs/error-codes.md`, `docs/log-search.md`, `docs/issues.md` |
| API keys and sessions | `docs/api-keys.md`, `docs/admin-sessions.md` |
| subscriptions and abuse guard | `docs/subscription-guard.md` |
| release gates | `docs/release-checklist.md` |
| machine-readable API | `web/openapi.json`, runtime `/openapi.json` |
