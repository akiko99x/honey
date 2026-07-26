# Docker deployment runbook

Docker Compose is the preferred clean-install deployment beginning with
v0.0.6. The systemd installer remains supported as a recovery and compatibility
path during the transition.

## Topology

`master`, `agent`, and `caddy` use host networking. This preserves loopback
communication and lets the panel create arbitrary VPN listeners without
rewriting Docker port mappings. PostgreSQL remains on the Compose bridge and
publishes only to `127.0.0.1`.

Persistent state:

| State | Location |
|---|---|
| PostgreSQL | `honey_postgres_data` volume |
| master CA/certificates and config | `honey_master_config` volume |
| master runtime state | `honey_master_state` volume |
| agent certificate and core configs | `honey_agent_config` volume |
| agent runtime state | `honey_agent_state` volume |
| WireGuard configs | `honey_wireguard_config` volume |
| Caddy certificates/config | `honey_caddy_data`, `honey_caddy_config` |
| PostgreSQL dumps | `/opt/honey-docker/backups` |
| Compose secrets | `/opt/honey-docker/secrets` |

## Install and upgrade

For a clean amd64 Ubuntu/Debian server, point the panel domain at the host and
allow public TCP 80/443. The standalone installer downloads a checksummed
release, provisions Docker Engine with Compose v2 when necessary, and deploys
the complete stack:

```bash
curl -fsSL https://raw.githubusercontent.com/akiko99x/honey/main/scripts/install-docker.sh \
  -o /tmp/honey-install-docker.sh
sudo bash /tmp/honey-install-docker.sh
rm -f /tmp/honey-install-docker.sh
```

It prompts for:

1. the panel domain;
2. the initial owner username and password;
3. whether to enroll this server as the first VPN node.

No separate PostgreSQL, Caddy, master or agent installation command is needed.
When local-node enrollment is enabled, the installer detects the host's public
IPv4 address instead of registering `127.0.0.1`. For NAT, split-horizon DNS, or
hosts where public-IP detection is blocked, set `HONEY_NODE_ADDRESS` to the
address that the master and clients can actually reach.
To pin a release instead of selecting the latest published tag, add
`--version vX.Y.Z`.

After installation:

```bash
cd /opt/honey-docker
docker compose ps
curl -fsS https://panel.example.com/health

# Upgrade the existing deployment:
sudo /opt/honey-docker/scripts/install-docker.sh --upgrade
```

An upgrade first creates a database dump, then pulls the tagged images and
recreates services. The one-shot `migrate` service must complete successfully
before master starts.

Do not run the Compose deployment beside an active systemd honey deployment;
they share host ports. A failed fresh install can be resumed only after its
reported error is corrected. Never remove named volumes from a deployment that
contains data unless destruction is explicitly intended.

After the first image publication, verify that the `honey-master`,
`honey-agent`, and `honey-backup` GHCR packages are public. GitHub creates new
container packages as private by default; public visibility is required for
anonymous installs.

## Health and logs

```bash
cd /opt/honey-docker
docker compose ps
docker compose logs --tail=200 master
docker compose logs --tail=200 agent
docker compose logs --tail=200 caddy
curl -fsS http://127.0.0.1:8080/health
curl -fsS https://panel.example.com/health
```

## Backup and restore rehearsal

```bash
/opt/honey-docker/scripts/docker-backup.sh
ls -lh /opt/honey-docker/backups
/opt/honey-docker/scripts/docker-restore-check.sh \
  honey-YYYYMMDDTHHMMSSZ.dump
```

The backup container uses the PostgreSQL 17 client, matching the server major
version. It writes atomic dumps and SHA-256 files and applies the same retention
policy as the systemd timer.

## ACME

Caddy owns public TCP 80/443. Requests under
`/.well-known/acme-challenge/` are forwarded to the Honey agent gateway at
`127.0.0.1:9080`. The gateway serves Honey-managed Xray HTTP-01 challenges and
proxies sing-box challenges to `127.0.0.1:9082` for every inbound hostname;
other HTTP requests redirect to the panel's HTTPS endpoint.

## Security notes

- Compose secrets are mounted from root-only files.
- The master entrypoint reads those files before dropping to the unprivileged
  `honey` account; the Rust process and migration command never run as root.
- Master and agent use separate config/state volumes; the agent cannot read the
  master CA private key.
- Master and Caddy use read-only root filesystems with writable volumes/tmpfs.
- Agent drops all capabilities except `NET_ADMIN` and `NET_BIND_SERVICE`.
- PostgreSQL is not publicly exposed.
- Do not mount the Docker socket into any honey container.
- Do not run `docker compose down -v` unless destroying the deployment and all
  persistent state is explicitly intended.
