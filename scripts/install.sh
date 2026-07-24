#!/usr/bin/env bash
# Install a honey release archive or three already-built Linux binaries.
#
# Safe by default: existing configuration is preserved and services are not
# started unless --start is explicitly supplied.
set -euo pipefail

usage() {
	cat >&2 <<'EOF'
usage:
  install.sh [--start] [--enable] [--update-repo owner/repo] [--allow-unverified] <release.tar.gz>
  install.sh [--start] [--enable] [--update-repo owner/repo] <honey-master> <honey-agent> <honey-enroll>
  install.sh
EOF
	exit 2
}

[[ "${EUID}" -eq 0 ]] || { echo "run as root" >&2; exit 1; }
source_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# No positional arguments means the full interactive bootstrap.
if (($# == 0)); then
	exec "$source_root/scripts/bootstrap.sh"
fi

start=0
enable=0
allow_unverified=0
update_repo=""
while (($#)); do
	case "$1" in
		--start) start=1; shift ;;
		--enable) enable=1; shift ;;
		--update-repo) (($# >= 2)) || usage; update_repo="$2"; shift 2 ;;
		--allow-unverified) allow_unverified=1; shift ;;
		--help|-h) usage ;;
		*) break ;;
	esac
done
(($# >= 1)) || usage
[[ -z "$update_repo" || "$update_repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] ||
	{ echo "invalid GitHub repository: $update_repo" >&2; exit 2; }

tmp=""
cleanup() { [[ -z "$tmp" ]] || rm -rf "$tmp"; }
trap cleanup EXIT

if (($# == 1)) && [[ "$1" == *.tar.gz ]]; then
	archive="$1"
	[[ -f "$archive" ]] || { echo "release archive not found: $archive" >&2; exit 1; }
	if [[ -f "${archive}.sha256" ]]; then
		(
			cd "$(dirname "$archive")"
			sha256sum -c "$(basename "${archive}.sha256")"
		)
	elif ((!allow_unverified)); then
		echo "missing ${archive}.sha256; pass --allow-unverified only for a trusted local archive" >&2
		exit 1
	fi
	tmp="$(mktemp -d)"
	tar -xzf "$archive" -C "$tmp"
	root="$(find "$tmp" -mindepth 1 -maxdepth 1 -type d -name 'honey-*' -print -quit)"
	[[ -n "$root" ]] || { echo "archive does not contain a honey-* directory" >&2; exit 1; }
	MASTER_BIN="$root/bin/honey-master"
	AGENT_BIN="$root/bin/honey-agent"
	ENROLL_BIN="$root/bin/honey-enroll"
	install_root="$root"
elif (($# == 3)); then
	MASTER_BIN="$1"
	AGENT_BIN="$2"
	ENROLL_BIN="$3"
	install_root="$source_root"
else
	usage
fi

for binary in "$MASTER_BIN" "$AGENT_BIN" "$ENROLL_BIN"; do
	[[ -f "$binary" ]] || { echo "binary not found: $binary" >&2; exit 1; }
done

id honey >/dev/null 2>&1 ||
	useradd --system --home /var/lib/honey --create-home --shell /usr/sbin/nologin honey

# The master updater replaces its own inode after checksum verification. Keep
# this directory writable only by the service account and readable by root.
install -d -o honey -g honey -m 0750 /opt/honey/bin
install -d -o honey -g honey -m 0750 \
	/etc/honey /etc/honey/certs /etc/honey/master-certs \
	/etc/honey/sing-box /etc/honey/xray
install -d -o honey -g honey -m 0750 /var/lib/honey
install -d -o honey -g honey -m 0700 /var/lib/honey/backups
install -o honey -g honey -m 0755 "$MASTER_BIN" /opt/honey/bin/honey-master
install -o honey -g honey -m 0755 "$AGENT_BIN" /opt/honey/bin/honey-agent
install -o honey -g honey -m 0755 "$ENROLL_BIN" /opt/honey/bin/honey-enroll

for helper in backup-postgres.sh restore-check.sh gen-certs.sh; do
	install -o root -g root -m 0755 "$install_root/scripts/$helper" "/opt/honey/bin/$helper"
done
for unit in honey-master.service honey-agent.service honey-backup.service honey-backup.timer; do
	install -o root -g root -m 0644 "$install_root/deploy/systemd/$unit" "/etc/systemd/system/$unit"
done

[[ -f /etc/honey/master.env ]] ||
	install -o honey -g honey -m 0600 "$install_root/deploy/systemd/master.env.example" /etc/honey/master.env
[[ -f /etc/honey/agent.env ]] ||
	install -o honey -g honey -m 0600 "$install_root/deploy/systemd/agent.env.example" /etc/honey/agent.env
if [[ -n "$update_repo" ]]; then
	if grep -q '^HONEY_UPDATE_REPO=' /etc/honey/master.env; then
		sed -i "s|^HONEY_UPDATE_REPO=.*|HONEY_UPDATE_REPO=$update_repo|" /etc/honey/master.env
	else
		printf '\nHONEY_UPDATE_REPO=%s\n' "$update_repo" >> /etc/honey/master.env
	fi
	chown honey:honey /etc/honey/master.env
	chmod 0600 /etc/honey/master.env
fi

systemctl daemon-reload
if ((enable)); then
	systemctl enable honey-master.service honey-agent.service honey-backup.timer
fi
if ((start)); then
	runuser -u honey -- /bin/bash -c \
		'set -a; . /etc/honey/master.env; set +a; exec /opt/honey/bin/honey-master migrate'
	systemctl restart honey-master.service honey-agent.service
	systemctl start honey-backup.timer
fi

cat <<'EOF'
honey installed.
Configuration is preserved in /etc/honey.
Before first start, set DATABASE_URL, HONEY_SECRET_KEY, node identity and
core paths in the environment files. Use --enable --start only after that.
EOF
