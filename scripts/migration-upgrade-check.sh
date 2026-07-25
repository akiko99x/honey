#!/usr/bin/env bash
# Apply an older migration baseline to a scratch DB, register the SQLx checksums,
# then prove the current honey-master upgrades it to the latest schema.
set -euo pipefail

: "${ADMIN_DATABASE_URL:?set ADMIN_DATABASE_URL to a role that can create databases}"
HONEY_MASTER_BIN="${HONEY_MASTER_BIN:-./master/target/debug/honey-master}"
BASELINE_MIGRATIONS="${HONEY_BASELINE_MIGRATIONS:-12}"
[[ "$BASELINE_MIGRATIONS" =~ ^[1-9][0-9]*$ ]] || {
	echo "HONEY_BASELINE_MIGRATIONS must be a positive integer" >&2
	exit 2
}
[[ -x "$HONEY_MASTER_BIN" ]] || {
	echo "honey-master binary not executable: $HONEY_MASTER_BIN" >&2
	exit 1
}

scratch="honey_migration_upgrade_$$"
base="${ADMIN_DATABASE_URL%/*}"
database_url="${base}/${scratch}"

cleanup() {
	psql "$ADMIN_DATABASE_URL" -v ON_ERROR_STOP=1 \
		-c "DROP DATABASE IF EXISTS ${scratch} WITH (FORCE);" >/dev/null 2>&1 || true
}
trap cleanup EXIT

psql "$ADMIN_DATABASE_URL" -v ON_ERROR_STOP=1 \
	-c "CREATE DATABASE ${scratch};" >/dev/null
psql "$database_url" -v ON_ERROR_STOP=1 <<'SQL' >/dev/null
CREATE TABLE _sqlx_migrations (
  version BIGINT PRIMARY KEY,
  description TEXT NOT NULL,
  installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
  success BOOLEAN NOT NULL,
  checksum BYTEA NOT NULL,
  execution_time BIGINT NOT NULL
);
SQL

mapfile -t migrations < <(find master/migrations -maxdepth 1 -type f -name '*.sql' | sort)
if ((BASELINE_MIGRATIONS >= ${#migrations[@]})); then
	echo "baseline must leave at least one migration for the upgrade" >&2
	exit 2
fi

for ((index = 0; index < BASELINE_MIGRATIONS; index++)); do
	migration="${migrations[$index]}"
	name="$(basename "$migration" .sql)"
	version="${name%%_*}"
	description="${name#*_}"
	description="${description//_/ }"
	checksum="$(openssl dgst -sha384 -binary "$migration" | od -An -vtx1 | tr -d ' \n')"
	psql "$database_url" -v ON_ERROR_STOP=1 -f "$migration" >/dev/null
	psql "$database_url" -v ON_ERROR_STOP=1 \
		-v version="$((10#$version))" -v description="$description" -v checksum="$checksum" \
		<<'SQL' >/dev/null
INSERT INTO _sqlx_migrations(version, description, success, checksum, execution_time)
VALUES (:version, :'description', true, decode(:'checksum', 'hex'), 0);
SQL
done

DATABASE_URL="$database_url" "$HONEY_MASTER_BIN" migrate >/dev/null
expected="${#migrations[@]}"
actual="$(psql "$database_url" -tAc "SELECT count(*) FROM _sqlx_migrations WHERE success;")"
[[ "$actual" == "$expected" ]] || {
	echo "migration count mismatch: expected $expected, got $actual" >&2
	exit 1
}
for table in notify_channels node_groups node_group_nodes user_node_groups saved_views admin_login_events system_notifications admin_notification_reads; do
	psql "$database_url" -tAc \
		"SELECT 1 FROM information_schema.tables WHERE table_name = '${table}';" | grep -qx 1
done
for table in nodes inbounds users; do
	psql "$database_url" -tAc \
		"SELECT 1 FROM information_schema.columns WHERE table_name = '${table}' AND column_name = 'labels';" | grep -qx 1
done
echo "upgrade ok: ${BASELINE_MIGRATIONS} baseline migrations -> ${actual} current migrations"
