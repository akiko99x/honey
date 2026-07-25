#!/usr/bin/env bash
# Full single-server bootstrap for honey.
#
# This provisions the host dependencies (PostgreSQL, Caddy and common tools),
# downloads a verified honey release, creates the database and owner account,
# generates the master mTLS identity, configures the panel domain and starts
# the supervised services. By default it also installs verified sing-box/Xray
# releases and enrolls this host as the first serve-mode VPN node.
set -euo pipefail

usage() {
	cat >&2 <<'EOF'
usage: bootstrap.sh [--repo owner/repo] [--version latest|vX.Y.Z]
                    [--non-interactive] [--force]

Interactive mode asks only for the GitHub repository/release, panel domain and
path, owner username/password, and optional ACME email.
Non-interactive mode reads HONEY_PANEL_DOMAIN, HONEY_ADMIN_USERNAME and
HONEY_ADMIN_PASSWORD (the other HONEY_* variables are optional).
EOF
	exit 2
}

[[ "${EUID}" -eq 0 ]] || { echo "run as root" >&2; exit 1; }
for command in awk curl openssl sed tar; do
	command -v "$command" >/dev/null || {
		echo "missing required command: $command" >&2
		exit 1
	}
done

repo="${HONEY_UPDATE_REPO:-akiko99x/honey}"
version="${HONEY_RELEASE_VERSION:-latest}"
non_interactive=0
force=0
while (($#)); do
	case "$1" in
		--repo) (($# >= 2)) || usage; repo="$2"; shift 2 ;;
		--version) (($# >= 2)) || usage; version="$2"; shift 2 ;;
		--non-interactive) non_interactive=1; shift ;;
		--force) force=1; shift ;;
		--help|-h) usage ;;
		*) usage ;;
	esac
done

[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] ||
	{ echo "invalid GitHub repository: $repo" >&2; exit 2; }

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

if ((non_interactive)); then
	panel_domain="${HONEY_PANEL_DOMAIN:-}"
	panel_path="${HONEY_PANEL_PATH:-/panel}"
	admin_user="${HONEY_ADMIN_USERNAME:-}"
	admin_password="${HONEY_ADMIN_PASSWORD:-}"
	caddy_email="${HONEY_CADDY_EMAIL:-}"
	install_local_node="${HONEY_INSTALL_LOCAL_NODE:-1}"
else
	read -r -p "GitHub repository [$repo]: " answer
	[[ -z "$answer" ]] || repo="$answer"
	read -r -p "Release version [$version]: " answer
	[[ -z "$answer" ]] || version="$answer"
	read -r -p "Panel domain (for example panel.example.com): " panel_domain
	read -r -p "Panel path [/panel]: " panel_path
	panel_path="${panel_path:-/panel}"
	read -r -p "Owner username [owner]: " admin_user
	admin_user="${admin_user:-owner}"
	while :; do
		read -r -s -p "Owner password: " admin_password
		echo
		read -r -s -p "Repeat owner password: " password_again
		echo
		[[ -n "$admin_password" && "$admin_password" == "$password_again" ]] && break
		echo "passwords are empty or do not match; try again" >&2
	done
	read -r -p "ACME email for Caddy (optional): " caddy_email
	read -r -p "Install this server as the first VPN node? [Y/n]: " answer
	case "${answer:-y}" in
		y|Y|yes|YES) install_local_node=1 ;;
		*) install_local_node=0 ;;
	esac
fi

[[ "$panel_domain" =~ ^[A-Za-z0-9.-]+\.[A-Za-z]{2,}$ ]] ||
	{ echo "invalid panel domain: $panel_domain" >&2; exit 2; }
[[ "$panel_path" == /* && "$panel_path" != *'..'* && "$panel_path" =~ ^/[A-Za-z0-9._/-]*$ ]] ||
	{ echo "invalid panel path: $panel_path" >&2; exit 2; }
[[ "$admin_user" =~ ^[A-Za-z0-9._-]{1,64}$ ]] ||
	{ echo "invalid owner username: $admin_user" >&2; exit 2; }
[[ -n "$admin_password" ]] || { echo "owner password is required" >&2; exit 2; }
[[ "$install_local_node" == "0" || "$install_local_node" == "1" ]] ||
	{ echo "HONEY_INSTALL_LOCAL_NODE must be 0 or 1" >&2; exit 2; }
[[ "$version" == latest || "$version" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] ||
	{ echo "invalid release version: $version" >&2; exit 2; }

if [[ -e /etc/honey/master.env || -e /etc/caddy/Caddyfile ]] && ((!force)); then
	cat >&2 <<'EOF'
existing honey/Caddy configuration detected.
Use --force only after taking a backup; the installer will save timestamped
copies before writing the new configuration.
EOF
	exit 1
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
if [[ -e /etc/honey/master.env ]]; then
	cp -a /etc/honey/master.env "/etc/honey/master.env.bootstrap-$stamp"
fi
if [[ -e /etc/caddy/Caddyfile ]]; then
	cp -a /etc/caddy/Caddyfile "/etc/caddy/Caddyfile.bootstrap-$stamp"
fi

echo "[1/10] installing host dependencies"
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y ca-certificates curl openssl tar unzip python3 postgresql postgresql-client
systemctl enable --now postgresql

if ! command -v caddy >/dev/null 2>&1; then
	apt-get install -y debian-keyring debian-archive-keyring apt-transport-https gnupg
	curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' |
		gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
	curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' |
		tee /etc/apt/sources.list.d/caddy-stable.list >/dev/null
	chmod o+r /usr/share/keyrings/caddy-stable-archive-keyring.gpg
	chmod o+r /etc/apt/sources.list.d/caddy-stable.list
	apt-get update
	apt-get install -y caddy
fi

echo "[2/10] downloading and verifying honey release"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

download_verified_github_asset() {
	local gh_repo="$1" requested="$2" pattern="$3" output_dir="$4"
	local endpoint json_file tag
	if [[ "$requested" == "latest" ]]; then
		endpoint="https://api.github.com/repos/$gh_repo/releases/latest"
	else
		tag="$requested"
		[[ "$tag" == v* ]] || tag="v$tag"
		endpoint="https://api.github.com/repos/$gh_repo/releases/tags/$tag"
	fi
	json_file="$work/$(echo "$gh_repo" | tr '/' '-')-release.json"
	curl -fsSL --retry 3 -H 'Accept: application/vnd.github+json' \
		-o "$json_file" "$endpoint"
	mapfile -t asset_meta < <(
		python3 - "$json_file" "$pattern" <<'PY'
import json
import re
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    release = json.load(handle)
matches = [
    asset for asset in release.get("assets", [])
    if re.fullmatch(sys.argv[2], asset.get("name", ""))
]
if len(matches) != 1:
    raise SystemExit(f"expected one release asset matching {sys.argv[2]!r}, found {len(matches)}")
asset = matches[0]
print(asset["name"])
print(asset["browser_download_url"])
print(asset.get("digest") or "")
print(release.get("tag_name") or "")
PY
	)
	((${#asset_meta[@]} == 4)) || { echo "could not resolve $gh_repo release asset" >&2; exit 1; }
	local name="${asset_meta[0]}" url="${asset_meta[1]}" digest="${asset_meta[2]}"
	digest="${digest#sha256:}"
	[[ "$digest" =~ ^[0-9a-fA-F]{64}$ ]] ||
		{ echo "$gh_repo does not publish a SHA-256 digest for $name" >&2; exit 1; }
	install -d -m 0700 "$output_dir"
	curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$output_dir/$name" "$url"
	(
		cd "$output_dir"
		printf '%s  %s\n' "$digest" "$name" | sha256sum -c - >&2
	)
	printf '%s\n' "$output_dir/$name"
}

downloader="$work/install-release.sh"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -x "$script_dir/install-release.sh" ]]; then
	cp "$script_dir/install-release.sh" "$downloader"
else
	curl -fsSL --retry 3 --proto '=https' --tlsv1.2 \
		-o "$downloader" \
		"https://raw.githubusercontent.com/$repo/main/scripts/install-release.sh"
fi
chmod 0755 "$downloader"
install_args=(--repo "$repo" --version "$version")
bash "$downloader" "${install_args[@]}"

if ((install_local_node)); then
	echo "[3/10] downloading verified sing-box and Xray cores"
	singbox_archive="$(
		download_verified_github_asset \
			SagerNet/sing-box "${HONEY_SINGBOX_VERSION:-latest}" \
			'sing-box-[0-9A-Za-z.-]+-linux-amd64\.tar\.gz' "$work/sing-box"
	)"
	tar -xzf "$singbox_archive" -C "$work/sing-box"
	singbox_binary="$(find "$work/sing-box" -type f -name sing-box -print -quit)"
	[[ -n "$singbox_binary" ]] || { echo "sing-box binary missing from release" >&2; exit 1; }
	install -o root -g root -m 0755 "$singbox_binary" /usr/local/bin/sing-box

	xray_archive="$(
		download_verified_github_asset \
			XTLS/Xray-core "${HONEY_XRAY_VERSION:-latest}" \
			'Xray-linux-64\.zip' "$work/xray"
	)"
	unzip -q "$xray_archive" -d "$work/xray/unpacked"
	[[ -f "$work/xray/unpacked/xray" ]] || { echo "Xray binary missing from release" >&2; exit 1; }
	install -o root -g root -m 0755 "$work/xray/unpacked/xray" /usr/local/bin/xray
fi

echo "[4/10] creating PostgreSQL database"
db_password="${HONEY_DB_PASSWORD:-$(openssl rand -hex 24)}"
[[ "$db_password" =~ ^[A-Za-z0-9]+$ ]] ||
	{ echo "HONEY_DB_PASSWORD must contain only letters and digits" >&2; exit 2; }
db_url="postgres://honey:${db_password}@127.0.0.1:5432/honey"
runuser -u postgres -- psql -v ON_ERROR_STOP=1 \
	-v honey_password="$db_password" <<'SQL'
DO $$
BEGIN
	IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'honey') THEN
		CREATE ROLE honey LOGIN;
	END IF;
END
$$;
ALTER ROLE honey PASSWORD :'honey_password';
SQL
if ! runuser -u postgres -- psql -Atqc "SELECT 1 FROM pg_database WHERE datname='honey'" | grep -qx 1; then
	runuser -u postgres -- createdb -O honey honey
fi

echo "[5/10] writing master environment"
install -d -o honey -g honey -m 0750 /etc/honey /etc/honey/master-certs
secret_key="${HONEY_SECRET_KEY:-$(/opt/honey/bin/honey-master keygen)}"
cat > /etc/honey/master.env <<EOF
DATABASE_URL=$db_url
HONEY_SECRET_KEY=$secret_key
HONEY_CERTS_DIR=/etc/honey/master-certs
HONEY_API_LISTEN=127.0.0.1:8080
HONEY_DIAL_LISTEN=0.0.0.0:9443
HONEY_UPDATE_REPO=$repo
RUST_LOG=info
EOF
chown honey:honey /etc/honey/master.env
chmod 0600 /etc/honey/master.env

echo "[6/10] generating master mTLS identity"
/opt/honey/bin/gen-certs.sh bootstrap-local 127.0.0.1 /etc/honey/master-certs
chown -R honey:honey /etc/honey/master-certs
chmod 0600 /etc/honey/master-certs/*.key

echo "[7/10] migrating database and creating owner"
runuser -u honey -- env DATABASE_URL="$db_url" HONEY_SECRET_KEY="$secret_key" \
	/opt/honey/bin/honey-master migrate
runuser -u honey -- env DATABASE_URL="$db_url" HONEY_SECRET_KEY="$secret_key" \
	HONEY_ADMIN_PASSWORD="$admin_password" \
	/opt/honey/bin/honey-master admin add "$admin_user" --role owner
runuser -u honey -- env DATABASE_URL="$db_url" HONEY_SECRET_KEY="$secret_key" \
	/opt/honey/bin/honey-master domain add "${panel_domain}${panel_path}"

echo "[8/10] configuring Caddy and systemd"
install -d -o caddy -g caddy -m 0750 /etc/caddy
{
	if [[ -n "$caddy_email" ]]; then
		printf '{\n'
		printf '\temail %s\n' "$caddy_email"
		printf '}\n'
	fi
	cat <<'CADDY_HTTP'
http:// {
	handle /.well-known/acme-challenge/* {
		reverse_proxy 127.0.0.1:9080
	}
	handle {
		redir https://{host}{uri} permanent
	}
}
CADDY_HTTP
	printf '%s {\n' "$panel_domain"
	printf '    reverse_proxy 127.0.0.1:8080\n'
	printf '}\n'
} > /etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile
systemctl daemon-reload
systemctl enable honey-master.service
systemctl enable --now honey-master.service
systemctl enable caddy.service
systemctl restart caddy.service

echo "[9/10] checking local health"
for _ in $(seq 1 20); do
	if curl -fsS http://127.0.0.1:8080/health >/dev/null; then
		break
	fi
	sleep 1
done
curl -fsS http://127.0.0.1:8080/health >/dev/null

if ((install_local_node)); then
	echo "[10/10] enrolling the local VPN node"
	api_dir="$work/local-node-api"
	install -d -m 0700 "$api_dir"
	cookie_jar="$api_dir/cookies"
	ADMIN_USERNAME="$admin_user" ADMIN_PASSWORD="$admin_password" \
		python3 - "$api_dir/login.json" <<'PY'
import json
import os
import sys

with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump({
        "username": os.environ["ADMIN_USERNAME"],
        "password": os.environ["ADMIN_PASSWORD"],
    }, handle)
PY
	curl -fsS -c "$cookie_jar" \
		-H "Host: $panel_domain" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/login.json" \
		http://127.0.0.1:8080/auth/login > "$api_dir/login-response.json"

	local_name="local-$(hostname -s | tr -cs 'A-Za-z0-9._-' '-')"
	NODE_NAME="$local_name" python3 - "$api_dir/node-create.json" <<'PY'
import json
import os
import sys

with open(sys.argv[1], "w", encoding="utf-8") as handle:
    json.dump({
        "name": os.environ["NODE_NAME"],
        "address": "127.0.0.1",
        "transport": "serve",
        "grpc_port": 8443,
        "tls_server_name": "honey-agent",
        "monthly_cost_cents": 0,
    }, handle)
PY
	curl -fsS -b "$cookie_jar" \
		-H "Host: $panel_domain" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/node-create.json" \
		http://127.0.0.1:8080/nodes > "$api_dir/node.json"
	node_id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$api_dir/node.json")"

	printf '{"tls_server_name":"node-%s.honey"}\n' "$node_id" > "$api_dir/node-update.json"
	curl -fsS -X PATCH -b "$cookie_jar" \
		-H "Host: $panel_domain" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/node-update.json" \
		"http://127.0.0.1:8080/nodes/$node_id" >/dev/null

	printf '{"expires_in_minutes":30}\n' > "$api_dir/enrollment-create.json"
	curl -fsS -b "$cookie_jar" \
		-H "Host: $panel_domain" -H 'Content-Type: application/json' \
		--data-binary "@$api_dir/enrollment-create.json" \
		"http://127.0.0.1:8080/nodes/$node_id/enrollments" > "$api_dir/enrollment.json"
	enrollment_token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["token"])' "$api_dir/enrollment.json")"

	runuser -u honey -- /opt/honey/bin/honey-enroll \
		--master http://127.0.0.1:8080 \
		--token "$enrollment_token" \
		--listen 0.0.0.0:8443 \
		--force
	systemctl enable honey-agent.service
	systemctl restart honey-agent.service
fi

cat <<EOF

honey bootstrap complete.

Panel:       https://${panel_domain}${panel_path}/
Owner:       ${admin_user}
Master env:  /etc/honey/master.env
Database:    PostgreSQL database honey

Open TCP ports 80, 443 and (if using dial nodes) 9443 in your provider
firewall. The panel reverse proxy owns TCP 443; choose another TCP port for a
VPN inbound unless you deliberately configure an Xray fallback. UDP 443 remains
available for Hysteria2.
EOF
