#!/usr/bin/env bash
# Session-management smoke for an already-started disposable master.
# Do not run against a production owner account: revoke-others intentionally
# terminates every pre-existing session for the selected test account.
set -euo pipefail

BASE="${HONEY_BASE_URL:-http://127.0.0.1:8080}"
USERNAME="${HONEY_ADMIN_USERNAME:?set HONEY_ADMIN_USERNAME}"
PASSWORD="${HONEY_ADMIN_PASSWORD:?set HONEY_ADMIN_PASSWORD}"
TOTP="${HONEY_ADMIN_TOTP:-}"

for tool in curl jq mktemp; do
	command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done

jar_a="$(mktemp)"
jar_b="$(mktemp)"
cleanup() { rm -f "$jar_a" "$jar_b"; }
trap cleanup EXIT

login() {
	local jar="$1"
	local body
	body="$(jq -cn --arg username "$USERNAME" --arg password "$PASSWORD" --arg totp_code "$TOTP" '{username:$username,password:$password} + (if $totp_code == "" then {} else {totp_code:$totp_code} end)')"
	curl -fsS -c "$jar" -b "$jar" -H 'content-type: application/json' \
		-d "$body" "${BASE}/auth/login" >/dev/null
}

login "$jar_a"
login "$jar_b"

sessions="$(curl -fsS -b "$jar_a" "${BASE}/auth/sessions")"
jq -e 'length >= 2 and any(.[]; .current == true)' <<<"$sessions" >/dev/null
other_id="$(curl -fsS -b "$jar_b" "${BASE}/auth/sessions" | jq -r '.[] | select(.current == true) | .id')"
[[ -n "$other_id" ]] || { echo "second session not found" >&2; exit 1; }

history="$(curl -fsS -b "$jar_a" "${BASE}/auth/login-history")"
jq -e 'any(.[]; .outcome == "success")' <<<"$history" >/dev/null
jq -e '[.[] | keys[]] | any(. == "token_hash") | not' <<<"$sessions" >/dev/null

curl -fsS -b "$jar_a" -X DELETE "${BASE}/auth/sessions/${other_id}" >/dev/null
status="$(curl -sS -o /dev/null -w '%{http_code}' -b "$jar_b" "${BASE}/auth/me")"
[[ "$status" == 401 ]] || { echo "revoked session still works: $status" >&2; exit 1; }

login "$jar_b"
curl -fsS -b "$jar_a" -X POST "${BASE}/auth/sessions/revoke-others" \
	| jq -e '.revoked >= 1' >/dev/null
status="$(curl -sS -o /dev/null -w '%{http_code}' -b "$jar_b" "${BASE}/auth/me")"
[[ "$status" == 401 ]] || { echo "revoke-others left session active: $status" >&2; exit 1; }

current_id="$(curl -fsS -b "$jar_a" "${BASE}/auth/sessions" | jq -r '.[] | select(.current == true) | .id')"
curl -fsS -b "$jar_a" -X DELETE "${BASE}/auth/sessions/${current_id}" >/dev/null
status="$(curl -sS -o /dev/null -w '%{http_code}' -b "$jar_a" "${BASE}/auth/me")"
[[ "$status" == 401 ]] || { echo "current-session revoke failed: $status" >&2; exit 1; }

echo "honey admin sessions smoke: ok"
