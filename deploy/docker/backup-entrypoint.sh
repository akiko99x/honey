#!/usr/bin/env bash
set -euo pipefail

secret_file="${HONEY_POSTGRES_PASSWORD_FILE:-/run/secrets/postgres_password}"
[[ -r "$secret_file" ]] || {
	echo "required secret is not readable: $secret_file" >&2
	exit 1
}
db_password="$(tr -d '\r\n' <"$secret_file")"
db_host="${HONEY_POSTGRES_HOST:-postgres}"
db_port="${HONEY_POSTGRES_PORT:-5432}"
urlencode() {
	local value="$1" char encoded="" hex index
	LC_ALL=C
	for ((index = 0; index < ${#value}; index++)); do
		char="${value:index:1}"
		case "$char" in
		[A-Za-z0-9.~_-]) encoded+="$char" ;;
		*)
			printf -v hex '%%%02X' "'$char"
			encoded+="$hex"
			;;
		esac
	done
	printf '%s' "$encoded"
}
encoded_password="$(urlencode "$db_password")"
export DATABASE_URL="${DATABASE_URL:-postgres://honey:${encoded_password}@${db_host}:${db_port}/honey}"
export ADMIN_DATABASE_URL="${ADMIN_DATABASE_URL:-postgres://honey:${encoded_password}@${db_host}:${db_port}/postgres}"

case "${1:-loop}" in
loop)
	initial="${HONEY_BACKUP_INITIAL_DELAY_SECONDS:-300}"
	interval="${HONEY_BACKUP_INTERVAL_SECONDS:-86400}"
	[[ "$initial" =~ ^[0-9]+$ && "$interval" =~ ^[1-9][0-9]*$ ]] || {
		echo "backup delays must be non-negative integer seconds" >&2
		exit 2
	}
	sleep "$initial"
	while :; do
		if ! /usr/local/bin/backup-postgres.sh /backups; then
			echo "scheduled honey backup failed" >&2
		fi
		sleep "$interval"
	done
	;;
backup)
	exec /usr/local/bin/backup-postgres.sh /backups
	;;
restore-check)
	shift
	exec /usr/local/bin/restore-check.sh "$@"
	;;
*)
	exec "$@"
	;;
esac
