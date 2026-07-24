#!/usr/bin/env bash
# Recovery-code smoke for an already-started disposable master.
# It deliberately rotates codes and consumes one; never run against production.
set -euo pipefail

BASE="${HONEY_BASE_URL:-http://127.0.0.1:8080}"
DATABASE_URL="${DATABASE_URL:?set DATABASE_URL to the disposable master's database}"
USERNAME="${HONEY_ADMIN_USERNAME:?set HONEY_ADMIN_USERNAME}"
PASSWORD="${HONEY_ADMIN_PASSWORD:?set HONEY_ADMIN_PASSWORD}"
TOTP="${HONEY_ADMIN_TOTP:?set the current HONEY_ADMIN_TOTP code}"

for tool in curl jq psql mktemp; do
	command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done

jar_totp="$(mktemp)"
jar_recovery="$(mktemp)"
payload="$(mktemp)"
trap 'rm -f "$jar_totp" "$jar_recovery" "$payload"' EXIT

login_body() {
	jq -cn --arg username "$USERNAME" --arg password "$PASSWORD" --arg totp_code "${1:-}" --arg recovery_code "${2:-}" \
		'{username:$username,password:$password} + (if $totp_code == "" then {} else {totp_code:$totp_code} end) + (if $recovery_code == "" then {} else {recovery_code:$recovery_code} end)'
}

curl -fsS -c "$jar_totp" -b "$jar_totp" -H 'content-type: application/json' \
	-d "$(login_body "$TOTP")" "${BASE}/auth/login" >/dev/null

status="$(curl -fsS -b "$jar_totp" "${BASE}/auth/totp/recovery")"
jq -e '.enabled == true and (.remaining | type == "number")' <<<"$status" >/dev/null

curl -fsS -b "$jar_totp" -H 'content-type: application/json' \
	-d "$(jq -cn --arg code "$TOTP" '{code:$code}')" \
	"${BASE}/auth/totp/recovery/generate" >"$payload"
code="$(jq -r '.codes[0] // empty' "$payload")"
[[ ${#code} -eq 20 ]] || { echo "recovery code was not returned once" >&2; exit 1; }
jq -e '.codes | length == 10 and all(.[]; (length == 20))' "$payload" >/dev/null

curl -fsS -c "$jar_recovery" -b "$jar_recovery" -H 'content-type: application/json' \
	-d "$(login_body "" "$code")" "${BASE}/auth/login" >/dev/null

replay_status="$(curl -sS -o /dev/null -w '%{http_code}' -c /dev/null -b /dev/null \
	-H 'content-type: application/json' -d "$(login_body "" "$code")" "${BASE}/auth/login")"
[[ "$replay_status" == 401 ]] || { echo "replayed recovery code was accepted: $replay_status" >&2; exit 1; }

bad_status="$(curl -sS -o /dev/null -w '%{http_code}' -c /dev/null -b /dev/null \
	-H 'content-type: application/json' -d "$(login_body "" "00000000000000000000")" "${BASE}/auth/login")"
[[ "$bad_status" == 401 ]] || { echo "invalid recovery code status: $bad_status" >&2; exit 1; }

history="$(curl -fsS -b "$jar_recovery" "${BASE}/auth/login-history")"
jq -e 'any(.[]; .outcome == "bad_recovery_code")' <<<"$history" >/dev/null
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -qAtc \
	"SELECT count(*) FROM admin_recovery_codes WHERE octet_length(code_hash) <> 32;" | grep -qx '0'

echo "honey admin recovery smoke: ok"
