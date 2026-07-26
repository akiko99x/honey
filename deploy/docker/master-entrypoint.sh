#!/usr/bin/env bash
set -euo pipefail

read_secret() {
	local path="$1"
	[[ -r "$path" ]] || {
		echo "required secret is not readable: $path" >&2
		exit 1
	}
	tr -d '\r\n' <"$path"
}

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

if [[ -z "${DATABASE_URL:-}" ]]; then
	db_password="$(read_secret "${HONEY_POSTGRES_PASSWORD_FILE:-/run/secrets/postgres_password}")"
	db_host="${HONEY_POSTGRES_HOST:-127.0.0.1}"
	db_port="${HONEY_POSTGRES_PORT:-5432}"
	export DATABASE_URL="postgres://honey:$(urlencode "$db_password")@${db_host}:${db_port}/honey"
	unset db_password
fi
if [[ -z "${HONEY_SECRET_KEY:-}" ]]; then
	export HONEY_SECRET_KEY="$(
		read_secret "${HONEY_SECRET_KEY_FILE:-/run/secrets/honey_master_key}"
	)"
fi
unset HONEY_SECRET_KEY_FILE
export HONEY_CERTS_DIR="${HONEY_CERTS_DIR:-/etc/honey/master-certs}"
if [[ -n "${HONEY_ADMIN_PASSWORD_FILE:-}" ]]; then
	export HONEY_ADMIN_PASSWORD="$(read_secret "$HONEY_ADMIN_PASSWORD_FILE")"
fi

if (($# == 0)); then
	set -- run --api-listen 127.0.0.1:8080 --dial-listen 0.0.0.0:9443
fi

run_as_honey() {
	if [[ "$(id -u)" -eq 0 ]]; then
		exec gosu honey "$@"
	fi
	exec "$@"
}

case "$1" in
honey-master)
	shift
	run_as_honey /usr/local/bin/honey-master "$@"
	;;
ping|migrate|keygen|reencrypt|rekey|admin|domain|push|serve|dial|run)
	run_as_honey /usr/local/bin/honey-master "$@"
	;;
*)
	# Release helpers such as gen-certs are explicit commands. They still run
	# as honey so named-volume files never become root-owned.
	run_as_honey "$@"
	;;
esac
