#!/usr/bin/env bash
# Runtime log search smoke for an already-started disposable master. It creates
# one failed login event and one authenticated session. Never use production.
set -euo pipefail

BASE="${HONEY_BASE_URL:-http://127.0.0.1:8080}"
USERNAME="${HONEY_ADMIN_USERNAME:?set HONEY_ADMIN_USERNAME}"
PASSWORD="${HONEY_ADMIN_PASSWORD:?set HONEY_ADMIN_PASSWORD}"
TOTP="${HONEY_ADMIN_TOTP:-}"

for tool in curl jq mktemp; do
	command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done

jar="$(mktemp)"
body_file="$(mktemp)"
cleanup() { rm -f "$jar" "$body_file"; }
trap cleanup EXIT

login="$(jq -cn --arg username "$USERNAME" --arg password "$PASSWORD" --arg totp_code "$TOTP" \
	'{username:$username,password:$password} + (if $totp_code == "" then {} else {totp_code:$totp_code} end)')"
curl -fsS -c "$jar" -b "$jar" -H 'content-type: application/json' \
	-d "$login" "${BASE}/auth/login" >/dev/null

request_id="smoke-log-${RANDOM}-${RANDOM}"
bad_password="must-not-appear-${RANDOM}"
bad_login="$(jq -cn --arg username "missing-log-smoke" --arg password "$bad_password" \
	'{username:$username,password:$password}')"
status="$(curl -sS -o /dev/null -w '%{http_code}' \
	-H 'content-type: application/json' -H "x-request-id: ${request_id}" \
	-d "$bad_login" "${BASE}/auth/login")"
[[ "$status" == 401 ]] || { echo "expected synthetic login HTTP 401, got $status" >&2; exit 1; }

sleep 1
curl -fsS -G -b "$jar" \
	--data-urlencode 'limit=20' \
	--data-urlencode 'level=warn' \
	--data-urlencode 'code=M0302' \
	--data-urlencode "q=${request_id}" \
	"${BASE}/system/logs" -o "$body_file"

jq -e --arg request_id "$request_id" \
	'any(.[]; .level == "warn" and .code == "M0302" and (.fields | contains($request_id)))' \
	"$body_file" >/dev/null
if grep -Fq "$bad_password" "$body_file"; then
	echo "runtime log API exposed a submitted password" >&2
	exit 1
fi

for query in 'level=fatal' 'code=N0406'; do
	status="$(curl -sS -o /dev/null -w '%{http_code}' -b "$jar" "${BASE}/system/logs?${query}")"
	[[ "$status" == 400 ]] || { echo "expected invalid filter HTTP 400, got $status" >&2; exit 1; }
done

long_query="$(printf 'x%.0s' {1..129})"
status="$(curl -sS -o /dev/null -w '%{http_code}' -G -b "$jar" \
	--data-urlencode "q=${long_query}" "${BASE}/system/logs")"
[[ "$status" == 400 ]] || { echo "expected oversized search HTTP 400, got $status" >&2; exit 1; }

echo "honey runtime log search smoke: ok"
