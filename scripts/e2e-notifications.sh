#!/usr/bin/env bash
# Persistent notification-center smoke for an already-started disposable master
# connected to DATABASE_URL. The script inserts synthetic operational events and
# marks all notifications read for the selected admin: never use production.
set -euo pipefail

BASE="${HONEY_BASE_URL:-http://127.0.0.1:8080}"
DATABASE_URL="${DATABASE_URL:?set DATABASE_URL to the disposable master's database}"
USERNAME="${HONEY_ADMIN_USERNAME:?set HONEY_ADMIN_USERNAME}"
PASSWORD="${HONEY_ADMIN_PASSWORD:?set HONEY_ADMIN_PASSWORD}"
TOTP="${HONEY_ADMIN_TOTP:-}"

for tool in curl jq mktemp psql; do
	command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done

jar="$(mktemp)"
ids=()
cleanup() {
	rm -f "$jar"
	if ((${#ids[@]})); then
		local joined
		joined="$(IFS=,; echo "${ids[*]}")"
		psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c \
			"DELETE FROM system_notifications WHERE id = ANY(string_to_array('${joined}', ',')::uuid[]);" >/dev/null || true
	fi
}
trap cleanup EXIT

body="$(jq -cn --arg username "$USERNAME" --arg password "$PASSWORD" --arg totp_code "$TOTP" '{username:$username,password:$password} + (if $totp_code == "" then {} else {totp_code:$totp_code} end)')"
curl -fsS -c "$jar" -b "$jar" -H 'content-type: application/json' \
	-d "$body" "${BASE}/auth/login" >/dev/null

insert_event() {
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -qAtc \
		"INSERT INTO system_notifications
		 (event_type,dedupe_key,severity,code,title,body,resource_type,resource_id)
		 VALUES ('node_down','smoke:' || gen_random_uuid(),'critical','M0409',
		         'Smoke node down','Synthetic disposable notification','node',gen_random_uuid()::text)
		 RETURNING id;"
}

first="$(insert_event)"
ids+=("$first")
before="$(curl -fsS -b "$jar" "${BASE}/notifications/unread-count" | jq -r '.unread')"
items="$(curl -fsS -b "$jar" "${BASE}/notifications?event=node_down&limit=200")"
jq -e --arg id "$first" 'any(.[]; .id == $id and .read_at == null and .severity == "critical" and .code == "M0409")' <<<"$items" >/dev/null
jq -e '[.[] | keys[]] | any(. == "dedupe_key") | not' <<<"$items" >/dev/null

curl -fsS -b "$jar" -X POST "${BASE}/notifications/${first}/read" >/dev/null
after="$(curl -fsS -b "$jar" "${BASE}/notifications/unread-count" | jq -r '.unread')"
[[ "$after" -eq $((before - 1)) ]] || { echo "unread count did not decrement: $before -> $after" >&2; exit 1; }

second="$(insert_event)"
ids+=("$second")
curl -fsS -b "$jar" -X POST "${BASE}/notifications/read-all" | jq -e '.marked >= 1' >/dev/null
curl -fsS -b "$jar" "${BASE}/notifications/unread-count" | jq -e '.unread == 0' >/dev/null
curl -fsS -b "$jar" "${BASE}/notifications?unread=true" | jq -e 'length == 0' >/dev/null

echo "honey in-app notifications smoke: ok"
