#!/usr/bin/env bash
# Install or upgrade honey as a Docker Compose application on Ubuntu/Debian.
set -euo pipefail
umask 077

usage() {
	cat >&2 <<'EOF'
usage: install-docker.sh [--repo owner/repo] [--version latest|vX.Y.Z]
                         [--install-dir /opt/honey-docker]
                         [--non-interactive] [--force] [--upgrade]

Fresh non-interactive installs require:
  HONEY_PANEL_DOMAIN, HONEY_ADMIN_USERNAME, HONEY_ADMIN_PASSWORD

Optional:
  HONEY_INSTALL_LOCAL_NODE=0|1, HONEY_NODE_ADDRESS, HONEY_POSTGRES_PORT,
  HONEY_BACKUP_DIR
EOF
	exit 2
}

[[ "${EUID}" -eq 0 ]] || { echo "run as root" >&2; exit 1; }
repo="${HONEY_UPDATE_REPO:-akiko99x/honey}"
requested="${HONEY_RELEASE_VERSION:-latest}"
install_dir="${HONEY_DOCKER_DIR:-/opt/honey-docker}"
non_interactive=0
force=0
upgrade=0
original_args=("$@")
while (($#)); do
	case "$1" in
	--repo) (($# >= 2)) || usage; repo="$2"; shift 2 ;;
	--version) (($# >= 2)) || usage; requested="$2"; shift 2 ;;
	--install-dir) (($# >= 2)) || usage; install_dir="$2"; shift 2 ;;
	--non-interactive) non_interactive=1; shift ;;
	--force) force=1; shift ;;
	--upgrade) upgrade=1; shift ;;
	-h|--help) usage ;;
	*) usage ;;
	esac
done

[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] ||
	{ echo "invalid GitHub repository: $repo" >&2; exit 2; }
[[ "$requested" == latest || "$requested" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] ||
	{ echo "invalid release version: $requested" >&2; exit 2; }
[[ "$install_dir" =~ ^/[A-Za-z0-9._/-]+$ && "$install_dir" != "/" ]] ||
	{ echo "install directory must be a safe absolute non-root path" >&2; exit 2; }

for command in curl openssl python3 sed sha256sum tar; do
	command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done

source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd || true)"
if [[ ! -f "$source_root/deploy/docker/compose.yml" ]]; then
	# A standalone copy of this script bootstraps itself from the verified
	# release archive, then re-executes the packaged installer.
	if [[ "$requested" == latest ]]; then
		tag="$(
			curl -fsSL --retry 3 -H 'Accept: application/vnd.github+json' \
				"https://api.github.com/repos/$repo/releases/latest" |
				sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' |
				head -n 1
		)"
	else
		tag="$requested"
		[[ "$tag" == v* ]] || tag="v$tag"
	fi
	[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] ||
		{ echo "could not resolve a valid honey release tag" >&2; exit 1; }
	version="${tag#v}"
	asset="honey-${version}-linux-amd64.tar.gz"
	tmp="$(mktemp -d)"
	trap 'rm -rf "$tmp"' EXIT
	base="https://github.com/$repo/releases/download/$tag"
	curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$tmp/$asset" "$base/$asset"
	curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$tmp/$asset.sha256" "$base/$asset.sha256"
	(cd "$tmp" && sha256sum -c "$asset.sha256")
	tar -xzf "$tmp/$asset" -C "$tmp"
	exec "$tmp/honey-${version}-linux-amd64/scripts/install-docker.sh" "${original_args[@]}"
fi

if [[ ! -r /etc/os-release ]]; then
	echo "cannot identify the operating system" >&2
	exit 1
fi
# shellcheck disable=SC1091
. /etc/os-release
case "${ID:-}:${ID_LIKE:-}" in
ubuntu:*|debian:*|*:debian*) ;;
*) echo "supported hosts are Ubuntu/Debian; found ${PRETTY_NAME:-unknown}" >&2; exit 1 ;;
esac

ensure_docker() {
	if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
		return
	fi
	echo "[1/8] installing Docker Engine and Compose"
	export DEBIAN_FRONTEND=noninteractive
	apt-get update
	compose_package=""
	for candidate in docker-compose-v2 docker-compose-plugin; do
		if apt-cache show "$candidate" >/dev/null 2>&1; then
			compose_package="$candidate"
			break
		fi
	done
	if [[ -n "$compose_package" ]]; then
		if command -v docker >/dev/null 2>&1; then
			apt-get install -y "$compose_package"
		else
			apt-get install -y docker.io "$compose_package"
		fi
	elif ! command -v docker >/dev/null 2>&1; then
		echo "adding Docker's official apt repository"
		[[ -n "${VERSION_CODENAME:-}" ]] || {
			echo "VERSION_CODENAME is missing from /etc/os-release" >&2
			exit 1
		}
		apt-get install -y ca-certificates curl
		install -m 0755 -d /etc/apt/keyrings
		curl -fsSL "https://download.docker.com/linux/${ID}/gpg" \
			-o /etc/apt/keyrings/docker.asc
		chmod a+r /etc/apt/keyrings/docker.asc
		cat >/etc/apt/sources.list.d/docker.sources <<EOF
Types: deb
URIs: https://download.docker.com/linux/${ID}
Suites: ${VERSION_CODENAME}
Components: stable
Architectures: $(dpkg --print-architecture)
Signed-By: /etc/apt/keyrings/docker.asc
EOF
		apt-get update
		apt-get install -y docker-ce docker-ce-cli containerd.io \
			docker-buildx-plugin docker-compose-plugin
	else
		echo "Docker is installed but Compose v2 is unavailable; install docker-compose-plugin" >&2
		exit 1
	fi
	systemctl enable --now docker
	docker compose version >/dev/null
}
if [[ ! -f "$install_dir/compose.yml" ]] &&
	systemctl is-active --quiet honey-master.service 2>/dev/null; then
	cat >&2 <<'EOF'
an active systemd honey deployment was detected.
Do not run the Docker stack beside it: both deployments use the same host
ports. Rehearse Docker on a clean server first, then migrate with a verified
database backup and an /etc/honey backup.
EOF
	exit 1
fi
ensure_docker

resolve_tag() {
	if [[ "$requested" == latest ]]; then
		curl -fsSL --retry 3 -H 'Accept: application/vnd.github+json' \
			"https://api.github.com/repos/$repo/releases/latest" |
			sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' |
			head -n 1
	else
		local tag="$requested"
		[[ "$tag" == v* ]] || tag="v$tag"
		printf '%s\n' "$tag"
	fi
}
tag="$(resolve_tag)"
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] ||
	{ echo "could not resolve a valid honey release tag" >&2; exit 1; }
version="${tag#v}"
image_prefix="ghcr.io/${repo,,}"

wait_for_health() {
	local panel_domain="$1"
	for _ in $(seq 1 60); do
		if curl -fsS http://127.0.0.1:8080/health >/dev/null; then
			break
		fi
		sleep 2
	done
	curl -fsS http://127.0.0.1:8080/health >/dev/null
	for _ in $(seq 1 60); do
		if curl -fsS "https://${panel_domain}/health" >/dev/null; then
			break
		fi
		sleep 2
	done
	curl -fsS "https://${panel_domain}/health" >/dev/null
}

if [[ -f "$install_dir/compose.yml" ]]; then
	[[ "$upgrade" -eq 1 || "$force" -eq 1 ]] || {
		echo "existing Docker deployment found at $install_dir; use --upgrade" >&2
		exit 1
	}
	echo "[2/8] backing up the current Docker deployment"
	(cd "$install_dir" && docker compose run --rm --no-deps backup backup)
	cp -a "$install_dir/compose.yml" "$install_dir/compose.yml.before-$version"
	cp -a "$source_root/deploy/docker/compose.yml" "$install_dir/compose.yml"
	cp -a "$source_root/deploy/docker/Caddyfile" "$install_dir/Caddyfile"
	install -d -m 0700 "$install_dir/scripts"
	install -m 0700 "$source_root/scripts/install-docker.sh" \
		"$source_root/scripts/docker-backup.sh" \
		"$source_root/scripts/docker-restore-check.sh" "$install_dir/scripts/"
	sed -i "s/^HONEY_VERSION=.*/HONEY_VERSION=$version/" "$install_dir/.env"
	sed -i "s|^HONEY_IMAGE_PREFIX=.*|HONEY_IMAGE_PREFIX=$image_prefix|" "$install_dir/.env"
	echo "[3/8] pulling honey $tag images"
	(cd "$install_dir" && docker compose pull)
	echo "[4/8] migrating and recreating containers"
	(cd "$install_dir" && docker compose up -d --remove-orphans)
	# Bind-mounted Caddyfile content is not part of Compose's service hash, so
	# explicitly reload it even when the Caddy image itself did not change.
	(cd "$install_dir" && docker compose restart caddy)
	panel_domain="$(
		sed -n 's/^HONEY_PANEL_DOMAIN=//p' "$install_dir/.env" | tail -n 1
	)"
	[[ "$panel_domain" =~ ^[A-Za-z0-9.-]+\.[A-Za-z]{2,}$ ]] || {
		echo "invalid or missing HONEY_PANEL_DOMAIN in $install_dir/.env" >&2
		exit 1
	}
	echo "[5/8] checking upgraded container health"
	(cd "$install_dir" && docker compose ps)
	wait_for_health "$panel_domain"
	echo "honey Docker deployment upgraded to $tag"
	exit 0
fi

if ((non_interactive)); then
	panel_domain="${HONEY_PANEL_DOMAIN:-}"
	admin_user="${HONEY_ADMIN_USERNAME:-}"
	admin_password="${HONEY_ADMIN_PASSWORD:-}"
	install_local_node="${HONEY_INSTALL_LOCAL_NODE:-1}"
else
	read -r -p "Panel domain (for example panel.example.com): " panel_domain
	read -r -p "Owner username [owner]: " admin_user
	admin_user="${admin_user:-owner}"
	while :; do
		read -r -s -p "Owner password: " admin_password; echo
		read -r -s -p "Repeat owner password: " password_again; echo
		[[ -n "$admin_password" && "$admin_password" == "$password_again" ]] && break
		echo "passwords are empty or do not match; try again" >&2
	done
	read -r -p "Install this server as the first VPN node? [Y/n]: " answer
	case "${answer:-y}" in y|Y|yes|YES) install_local_node=1 ;; *) install_local_node=0 ;; esac
fi
[[ "$panel_domain" =~ ^[A-Za-z0-9.-]+\.[A-Za-z]{2,}$ ]] ||
	{ echo "invalid panel domain: $panel_domain" >&2; exit 2; }
[[ "$admin_user" =~ ^[A-Za-z0-9._-]{1,64}$ && -n "$admin_password" ]] ||
	{ echo "valid owner username and password are required" >&2; exit 2; }
[[ "$install_local_node" == 0 || "$install_local_node" == 1 ]] ||
	{ echo "HONEY_INSTALL_LOCAL_NODE must be 0 or 1" >&2; exit 2; }
postgres_port="${HONEY_POSTGRES_PORT:-5432}"
backup_dir="${HONEY_BACKUP_DIR:-$install_dir/backups}"
backup_keep="${HONEY_BACKUP_KEEP:-14}"
backup_initial_delay="${HONEY_BACKUP_INITIAL_DELAY_SECONDS:-300}"
backup_interval="${HONEY_BACKUP_INTERVAL_SECONDS:-86400}"
[[ "$postgres_port" =~ ^[0-9]+$ ]] &&
	((10#$postgres_port >= 1 && 10#$postgres_port <= 65535)) ||
	{ echo "HONEY_POSTGRES_PORT must be between 1 and 65535" >&2; exit 2; }
[[ "$backup_dir" =~ ^/[A-Za-z0-9._/-]+$ && "$backup_dir" != "/" ]] ||
	{ echo "HONEY_BACKUP_DIR must be a safe absolute non-root path" >&2; exit 2; }
[[ "$backup_keep" =~ ^[1-9][0-9]*$ ]] ||
	{ echo "HONEY_BACKUP_KEEP must be a positive integer" >&2; exit 2; }
[[ "$backup_initial_delay" =~ ^[0-9]+$ ]] ||
	{ echo "HONEY_BACKUP_INITIAL_DELAY_SECONDS must be a non-negative integer" >&2; exit 2; }
[[ "$backup_interval" =~ ^[1-9][0-9]*$ ]] ||
	{ echo "HONEY_BACKUP_INTERVAL_SECONDS must be a positive integer" >&2; exit 2; }
if command -v ss >/dev/null 2>&1; then
	conflicting_listeners="$(
		ss -H -lntup 2>/dev/null |
			grep -E ":($postgres_port|80|443|8080|8081|8443|9080|9082|9090|9443)\\b" ||
			true
	)"
	if [[ -n "$conflicting_listeners" ]]; then
		cat >&2 <<EOF
fixed honey ports are already in use:
$conflicting_listeners
stop or reconfigure the conflicting service before a fresh Docker install.
EOF
		exit 1
	fi
fi

runtime_tmp="$(mktemp -d)"
trap 'rm -rf "$runtime_tmp"' EXIT

echo "[2/8] writing Docker deployment"
install -d -m 0700 "$install_dir" "$install_dir/secrets" \
	"$install_dir/backups" "$install_dir/scripts"
install -m 0600 "$source_root/deploy/docker/compose.yml" "$install_dir/compose.yml"
install -m 0600 "$source_root/deploy/docker/Caddyfile" "$install_dir/Caddyfile"
install -m 0700 "$source_root/scripts/install-docker.sh" \
	"$source_root/scripts/docker-backup.sh" \
	"$source_root/scripts/docker-restore-check.sh" "$install_dir/scripts/"
postgres_password="${HONEY_DB_PASSWORD:-$(openssl rand -hex 24)}"
printf '%s\n' "$postgres_password" >"$install_dir/secrets/postgres_password"
openssl rand -base64 32 | tr -d '\n' >"$install_dir/secrets/honey_master_key"
printf '\n' >>"$install_dir/secrets/honey_master_key"
chmod 0600 "$install_dir/secrets/"*
cat >"$install_dir/.env" <<EOF
HONEY_VERSION=$version
HONEY_IMAGE_PREFIX=$image_prefix
HONEY_PANEL_DOMAIN=$panel_domain
HONEY_UPDATE_REPO=$repo
HONEY_POSTGRES_BIND=127.0.0.1
HONEY_POSTGRES_PORT=$postgres_port
HONEY_BACKUP_DIR=$backup_dir
HONEY_BACKUP_KEEP=$backup_keep
HONEY_BACKUP_INITIAL_DELAY_SECONDS=$backup_initial_delay
HONEY_BACKUP_INTERVAL_SECONDS=$backup_interval
COMPOSE_PROFILES=
EOF
chmod 0600 "$install_dir/.env"

cd "$install_dir"
echo "[3/8] pulling honey $tag images"
docker compose pull
echo "[4/8] starting PostgreSQL and applying migrations"
docker compose up -d postgres
docker compose run --rm migrate

echo "[5/8] creating master PKI and owner"
docker compose run --rm --no-deps \
	master /usr/local/bin/gen-certs.sh \
	bootstrap-local 127.0.0.1 /etc/honey/master-certs
printf '%s\n' "$admin_password" >"$runtime_tmp/admin_password"
chmod 0600 "$runtime_tmp/admin_password"
unset admin_password password_again
docker compose run --rm --no-deps \
	-e HONEY_ADMIN_PASSWORD_FILE=/run/secrets/bootstrap_admin_password \
	-v "$runtime_tmp/admin_password:/run/secrets/bootstrap_admin_password:ro" master \
	admin add "$admin_user" --role owner
docker compose run --rm --no-deps master domain add "${panel_domain}/panel"

echo "[6/8] starting master, Caddy and scheduled backups"
docker compose up -d master caddy backup

if [[ "$install_local_node" == 1 ]]; then
	echo "[7/8] enrolling the local VPN node"
	node_address="${HONEY_NODE_ADDRESS:-}"
	if [[ -z "$node_address" ]]; then
		node_address="$(curl -4fsSL --retry 3 --connect-timeout 10 \
			https://api.ipify.org 2>/dev/null || true)"
	fi
	if [[ -z "$node_address" || "$node_address" == 127.* || "$node_address" == localhost ]]; then
		echo "could not determine a public node address; set HONEY_NODE_ADDRESS and retry" >&2
		exit 1
	fi
	api_dir="$runtime_tmp/api"
	install -d -m 0700 "$api_dir"
	cookie_jar="$api_dir/cookies"
	ADMIN_USERNAME="$admin_user" \
		python3 - "$api_dir/login.json" "$runtime_tmp/admin_password" <<'PY'
import json, os, sys
with open(sys.argv[2], "r", encoding="utf-8") as source:
    password = source.read().rstrip("\r\n")
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump({"username": os.environ["ADMIN_USERNAME"],
               "password": password}, handle)
PY
	curl -fsS -c "$cookie_jar" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/login.json" \
		http://127.0.0.1:8080/auth/login >/dev/null
	rm -f "$api_dir/login.json" "$runtime_tmp/admin_password"
	local_name="local-$(hostname -s | tr -cs 'A-Za-z0-9._-' '-')"
	NODE_NAME="$local_name" NODE_ADDRESS="$node_address" python3 - "$api_dir/node.json" <<'PY'
import json, os, sys
with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump({"name": os.environ["NODE_NAME"], "address": os.environ["NODE_ADDRESS"],
               "transport": "serve", "grpc_port": 8443,
               "tls_server_name": "honey-agent", "monthly_cost_cents": 0}, handle)
PY
	curl -fsS -b "$cookie_jar" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/node.json" \
		http://127.0.0.1:8080/nodes >"$api_dir/node-response.json"
	node_id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$api_dir/node-response.json")"
	printf '{"tls_server_name":"node-%s.honey"}\n' "$node_id" >"$api_dir/update.json"
	curl -fsS -X PATCH -b "$cookie_jar" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/update.json" \
		"http://127.0.0.1:8080/nodes/$node_id" >/dev/null
	printf '{"expires_in_minutes":30}\n' >"$api_dir/enrollment.json"
	curl -fsS -b "$cookie_jar" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/enrollment.json" \
		"http://127.0.0.1:8080/nodes/$node_id/enrollments" >"$api_dir/enrollment-response.json"
	token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["token"])' "$api_dir/enrollment-response.json")"
	docker compose --profile node run --rm --no-deps \
		--entrypoint /usr/local/bin/honey-enroll agent \
		--master http://127.0.0.1:8080 --token "$token" \
		--listen 0.0.0.0:8443 --force
	sed -i 's/^COMPOSE_PROFILES=.*/COMPOSE_PROFILES=node/' "$install_dir/.env"
	docker compose --profile node up -d agent
else
	echo "[7/8] local VPN node intentionally skipped"
	rm -f "$runtime_tmp/admin_password"
fi

echo "[8/8] checking container health"
docker compose ps
wait_for_health "$panel_domain"
cat <<EOF

honey Docker deployment $tag is ready.

Panel:      https://${panel_domain}/panel/
Directory:  ${install_dir}
Backups:    ${install_dir}/backups

Upgrade:
  $install_dir/scripts/install-docker.sh --upgrade
EOF
