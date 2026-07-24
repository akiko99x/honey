#!/usr/bin/env bash
# Prove that an encrypted honey database backup is useful only together with
# HONEY_SECRET_KEY, and that restored data survives a real key rotation.
set -euo pipefail
umask 077

: "${DATABASE_URL:?set DATABASE_URL}"
: "${ADMIN_DATABASE_URL:?set ADMIN_DATABASE_URL (must create/drop databases)}"
: "${HONEY_SECRET_KEY:?set HONEY_SECRET_KEY}"

MASTER="${HONEY_MASTER_BIN:-master/target/debug/honey-master}"
BASE="${HONEY_SECRET_CHECK_URL:-http://127.0.0.1:18081}"
LISTEN="${HONEY_SECRET_CHECK_LISTEN:-127.0.0.1:18081}"
TOKEN="secret-recovery-$RANDOM-$$"
scratch="honey_secret_recovery_$$"
work="$(mktemp -d)"
master_pid=""
user_id=""

need() { command -v "$1" >/dev/null || { echo "missing tool: $1" >&2; exit 1; }; }
for tool in curl jq psql pg_restore; do need "$tool"; done
[[ -x "$MASTER" ]] || { echo "master binary is not executable: $MASTER" >&2; exit 1; }

admin_base="${ADMIN_DATABASE_URL%/*}"
restored_url="${admin_base}/${scratch}"

cleanup() {
	[[ -z "$master_pid" ]] || kill "$master_pid" 2>/dev/null || true
	[[ -z "$master_pid" ]] || wait "$master_pid" 2>/dev/null || true
	[[ -z "$user_id" ]] || psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "DELETE FROM users WHERE id = '${user_id}'" >/dev/null 2>&1 || true
	psql "$ADMIN_DATABASE_URL" -c "DROP DATABASE IF EXISTS ${scratch}" >/dev/null 2>&1 || true
	rm -rf -- "$work"
}
trap cleanup EXIT

HONEY_API_TOKEN="$TOKEN" HONEY_SECRET_KEY="$HONEY_SECRET_KEY" \
	"$MASTER" serve --database-url "$DATABASE_URL" --listen "$LISTEN" \
	>"$work/master.log" 2>&1 &
master_pid=$!
for _ in $(seq 1 40); do
	if curl -fsS "$BASE/ready" >/dev/null 2>&1; then break; fi
	if ! kill -0 "$master_pid" 2>/dev/null; then
		cat "$work/master.log" >&2
		exit 1
	fi
	sleep 0.25
done
curl -fsS "$BASE/ready" >/dev/null

user="$(curl -fsS "$BASE/users" \
	-H "authorization: Bearer ${TOKEN}" -H 'content-type: application/json' \
	-d "{\"username\":\"secret-recovery-$$\",\"password\":\"recovery-password-$$\"}")"
user_id="$(jq -r .id <<<"$user")"

raw="$(psql "$DATABASE_URL" -Atc "SELECT uuid || E'\\n' || password FROM users WHERE id = '${user_id}'")"
count="$(grep -c '^enc:v1:' <<<"$raw")"
[[ "$count" == 2 ]] || { echo "test credentials were not encrypted at rest" >&2; exit 1; }

backup_file="$(HONEY_BACKUP_KEEP=1 bash "$(dirname "$0")/backup-postgres.sh" "$work/backups")"
kill "$master_pid" 2>/dev/null || true
wait "$master_pid" 2>/dev/null || true
master_pid=""

psql "$ADMIN_DATABASE_URL" -v ON_ERROR_STOP=1 -c "CREATE DATABASE ${scratch}" >/dev/null
pg_restore --no-owner --no-acl --dbname="$restored_url" "$backup_file" >/dev/null

HONEY_SECRET_KEY="$HONEY_SECRET_KEY" "$MASTER" reencrypt --database-url "$restored_url" >/dev/null
wrong_key="$("$MASTER" keygen)"
if HONEY_SECRET_KEY="$wrong_key" "$MASTER" reencrypt --database-url "$restored_url" >"$work/wrong-key.log" 2>&1; then
	echo "wrong HONEY_SECRET_KEY unexpectedly decrypted the restored database" >&2
	exit 1
fi

new_key="$("$MASTER" keygen)"
HONEY_SECRET_KEY_OLD="$HONEY_SECRET_KEY" HONEY_SECRET_KEY="$new_key" \
	"$MASTER" rekey --database-url "$restored_url" >/dev/null
HONEY_SECRET_KEY="$new_key" "$MASTER" reencrypt --database-url "$restored_url" >/dev/null
if HONEY_SECRET_KEY="$HONEY_SECRET_KEY" "$MASTER" reencrypt --database-url "$restored_url" >"$work/old-key.log" 2>&1; then
	echo "old HONEY_SECRET_KEY still decrypted data after rekey" >&2
	exit 1
fi

restored_count="$(psql "$restored_url" -Atc "SELECT count(*) FROM users WHERE id = '${user_id}' AND uuid LIKE 'enc:v1:%' AND password LIKE 'enc:v1:%'")"
[[ "$restored_count" == 1 ]] || { echo "restored/rekeyed credentials are missing" >&2; exit 1; }

echo "honey secret recovery: ok (backup + correct/wrong key + rekey)"
