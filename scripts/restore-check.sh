#!/usr/bin/env bash
# Verify a honey backup actually restores: load it into a throwaway database,
# check the schema/rows are there, then drop it. Run this on a schedule so a
# backup is proven, not just produced.
#
#   ./restore-check.sh <backup.dump[.gpg]>
#   ADMIN_DATABASE_URL   a superuser/owner URL that can CREATE/DROP DATABASE
#                        (default: postgres://postgres@127.0.0.1/postgres)
set -euo pipefail

file="${1:?usage: restore-check.sh <backup.dump[.gpg]>}"
admin_url="${ADMIN_DATABASE_URL:-postgres://postgres@127.0.0.1/postgres}"
scratch="honey_restore_check_$$"

# verify the checksum if present.
if [[ -f "${file}.sha256" ]]; then
	(cd "$(dirname "$file")" && sha256sum -c "$(basename "$file").sha256")
fi

dump="$file"
tmp=""
if [[ "$file" == *.gpg ]]; then
	tmp="$(mktemp --suffix=.dump)"
	gpg --yes --batch --decrypt --output "$tmp" "$file"
	dump="$tmp"
fi

cleanup() {
	psql "$admin_url" -c "DROP DATABASE IF EXISTS ${scratch};" >/dev/null 2>&1 || true
	[[ -n "$tmp" ]] && rm -f "$tmp"
}
trap cleanup EXIT

psql "$admin_url" -c "CREATE DATABASE ${scratch};" >/dev/null
base="${admin_url%/*}"
pg_restore --no-owner --no-acl --dbname="${base}/${scratch}" "$dump" >/dev/null

users="$(psql "${base}/${scratch}" -tAc "SELECT count(*) FROM users;")"
nodes="$(psql "${base}/${scratch}" -tAc "SELECT count(*) FROM nodes;")"
echo "restore ok: ${users} users, ${nodes} nodes in a scratch database"
