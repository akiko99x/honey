# honey Docker deployment

This directory is the production Docker Compose deployment for honey. It keeps
the former systemd deployment available as a fallback, but all honey
application processes run in containers:

- PostgreSQL 17 with a persistent named volume;
- honey master and migration job;
- honey agent with bundled sing-box and Xray;
- Caddy on the host network;
- a PostgreSQL 17 backup container.

Master, agent and Caddy use host networking intentionally. The panel and ACME
gateway continue to use loopback, while dynamically-created VPN inbound ports
do not require Compose file changes. PostgreSQL alone publishes its port on
`127.0.0.1`.

## Install

Run the installer from a published release:

```bash
curl -fsSL https://raw.githubusercontent.com/akiko99x/honey/main/scripts/install-docker.sh \
  -o /tmp/honey-install-docker.sh
sudo bash /tmp/honey-install-docker.sh
rm -f /tmp/honey-install-docker.sh
```

This one script can provision Docker Compose and deploy PostgreSQL, migrations,
master, Caddy, scheduled backups and an optional local VPN node. It prompts for
the panel domain, initial owner credentials and whether to enroll the host as
that node. Use `--version vX.Y.Z` to pin a release.

Non-interactive:

```bash
curl -fsSL https://raw.githubusercontent.com/akiko99x/honey/main/scripts/install-docker.sh \
  -o /tmp/honey-install-docker.sh
sudo env \
  HONEY_PANEL_DOMAIN=panel.example.com \
  HONEY_ADMIN_USERNAME=owner \
  HONEY_ADMIN_PASSWORD='replace-me' \
HONEY_INSTALL_LOCAL_NODE=1 \
  bash /tmp/honey-install-docker.sh --non-interactive
rm -f /tmp/honey-install-docker.sh
```

For a local node, the installer detects the host's public IPv4 address before
creating the node record. If the server is behind NAT or outbound detection is
blocked, set `HONEY_NODE_ADDRESS` explicitly; it must be the address clients
and the master can use to reach the node, not `127.0.0.1`.

The deployment lives in `/opt/honey-docker` by default. Secrets are ordinary
root-only files consumed through Compose secrets; they are not stored in the
Compose `.env` file. The master entrypoint reads them before dropping
privileges, and the application receives only the runtime values it needs.

The three GHCR packages (`honey-master`, `honey-agent`, and `honey-backup`)
must be public so a fresh host can pull them without GitHub credentials. Check
their package visibility after the first container release; GHCR package
visibility is separate from repository visibility.

## Operations

```bash
cd /opt/honey-docker
docker compose ps
docker compose logs -f master agent caddy
./scripts/docker-backup.sh
./scripts/docker-restore-check.sh honey-YYYYMMDDTHHMMSSZ.dump
./scripts/install-docker.sh --upgrade
```

Never use `docker compose down --volumes` on a production deployment. That
explicitly deletes PostgreSQL, configuration, certificate and Caddy volumes.

## Host requirements

- Ubuntu or Debian on amd64;
- public TCP 80/443 for Caddy;
- any VPN TCP/UDP ports configured in the panel;
- TCP 9443 only when dial-mode agents use the master acceptor;
- Docker Engine with Compose v2.

The agent keeps only `NET_ADMIN` and `NET_BIND_SERVICE`, but runs as root inside
its container because nftables, WireGuard helpers and arbitrary low inbound
ports are part of the node runtime. The other long-running services run without
these capabilities. Master and agent configuration volumes are separate, so
the node container cannot read the master CA private key.

The master container entrypoint starts as root only long enough to read
root-owned Compose secrets, then uses `gosu` before invoking `honey-master` or
release helpers. The application and migrations run as the `honey` user.

Caddy accepts HTTP-01 challenge requests for every hostname on this server and
forwards them to the local Honey ACME gateway; other HTTP requests redirect to
the panel's HTTPS endpoint.

## Legacy migration

Do not start this Compose stack alongside an active systemd honey deployment:
they share ports 80, 443, 8080, 8443, 9080 and 9443. Before migrating, take a
database backup and copy `/etc/honey`. The initial Docker release is intended
for clean-server rehearsal; migration of an existing production host should be
performed from the documented backup/restore runbook after validating the new
stack on a disposable server.
