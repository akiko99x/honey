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

```bash
sudo bash scripts/install-docker.sh
sudo /opt/honey-docker/scripts/install-docker.sh --upgrade
```

An upgrade first creates a database dump, then pulls the tagged images and
recreates services. The one-shot `migrate` service must complete successfully
before master starts.

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
proxies sing-box challenges to `127.0.0.1:9082`.

## Security notes

- Compose secrets are mounted from root-only files.
- Master and agent use separate config/state volumes; the agent cannot read the
  master CA private key.
- Master and Caddy use read-only root filesystems with writable volumes/tmpfs.
- Agent drops all capabilities except `NET_ADMIN` and `NET_BIND_SERVICE`.
- PostgreSQL is not publicly exposed.
- Do not mount the Docker socket into any honey container.
- Do not run `docker compose down -v` unless destroying the deployment and all
  persistent state is explicitly intended.
