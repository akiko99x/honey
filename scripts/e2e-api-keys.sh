#!/usr/bin/env bash
# Scoped API-key lifecycle smoke for an already-started disposable master.
# It creates and revokes real keys. Never point it at production.
set -euo pipefail

BASE="${HONEY_BASE_URL:-http://127.0.0.1:8080}"
USERNAME="${HONEY_ADMIN_USERNAME:?set HONEY_ADMIN_USERNAME to a disposable owner}"
PASSWORD="${HONEY_ADMIN_PASSWORD:?set HONEY_ADMIN_PASSWORD}"
TOTP="${HONEY_ADMIN_TOTP:-}"

for tool in curl jq mktemp; do
	command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done

jar="$(mktemp)"
body="$(mktemp)"
created_ids=()
cleanup() {
	for id in "${created_ids[@]:-}"; do
		curl -fsS -b "$jar" -X DELETE "${BASE}/api-keys/${id}" >/dev/null 2>&1 || true
	done
	rm -f "$jar" "$body"
}
trap cleanup EXIT

login="$(jq -cn --arg username "$USERNAME" --arg password "$PASSWORD" --arg totp_code "$TOTP" \
	'{username:$username,password:$password} + (if $totp_code == "" then {} else {totp_code:$totp_code} end)')"
curl -fsS -c "$jar" -b "$jar" -H 'content-type: application/json' \
	-d "$login" "${BASE}/auth/login" >/dev/null

create_key() {
	local name="$1" role="$2" days="$3"
	curl -fsS -b "$jar" -H 'content-type: application/json' \
		-d "$(jq -cn --arg name "$name" --arg role "$role" --argjson days "$days" \
		'{name:$name,role:$role,expires_days:$days}')" \
		"${BASE}/api-keys"
}

viewer="$(create_key "smoke-viewer-${RANDOM}" viewer 1)"
viewer_id="$(jq -er '.id' <<<"$viewer")"
viewer_token="$(jq -er '.token | select(startswith("hny_"))' <<<"$viewer")"
created_ids+=("$viewer_id")
jq -e '.status == "active" and .role == "viewer" and .expires_at != null' <<<"$viewer" >/dev/null

curl -fsS -H "Authorization: Bearer ${viewer_token}" "${BASE}/nodes" >/dev/null
status="$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer ${viewer_token}" "${BASE}/api-keys")"
[[ "$status" == 403 ]] || { echo "viewer listed API keys: HTTP $status" >&2; exit 1; }
status="$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer ${viewer_token}" \
	-H 'content-type: application/json' -d '{}' "${BASE}/nodes")"
[[ "$status" == 403 ]] || { echo "viewer mutated nodes: HTTP $status" >&2; exit 1; }

admin="$(create_key "smoke-admin-${RANDOM}" admin 0)"
admin_id="$(jq -er '.id' <<<"$admin")"
admin_token="$(jq -er '.token' <<<"$admin")"
created_ids+=("$admin_id")
status="$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer ${admin_token}" "${BASE}/api-keys")"
[[ "$status" == 403 ]] || { echo "admin managed owner-only API keys: HTTP $status" >&2; exit 1; }

curl -fsS -b "$jar" "${BASE}/api-keys" -o "$body"
jq -e --arg id "$viewer_id" 'any(.[]; .id == $id and .status == "active" and .last_used_at != null)' "$body" >/dev/null
jq -e 'all(.[]; has("token") | not) and all(.[]; has("key_hash") | not)' "$body" >/dev/null

for payload in \
	'{"name":"bad-negative","role":"viewer","expires_days":-1}' \
	'{"name":"bad-long","role":"viewer","expires_days":3651}' \
	'{"name":"bad-role","role":"reseller","expires_days":0}'; do
	status="$(curl -sS -o /dev/null -w '%{http_code}' -b "$jar" -H 'content-type: application/json' -d "$payload" "${BASE}/api-keys")"
	[[ "$status" == 400 ]] || { echo "invalid key input returned HTTP $status" >&2; exit 1; }
done

curl -fsS -b "$jar" -X DELETE "${BASE}/api-keys/${viewer_id}" >/dev/null
created_ids=("$admin_id")
status="$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer ${viewer_token}" "${BASE}/nodes")"
[[ "$status" == 401 ]] || { echo "revoked key remained active: HTTP $status" >&2; exit 1; }

curl -fsS -b "$jar" "${BASE}/api-keys" -o "$body"
jq -e --arg id "$viewer_id" 'any(.[]; .id == $id and .status == "revoked")' "$body" >/dev/null
curl -fsS "${BASE}/openapi.json" | jq -e '.openapi == "3.0.3" and .paths["/api-keys"] and .paths["/nodes"] and .components.schemas.Error' >/dev/null
curl -fsS -b "$jar" "${BASE}/audit" | jq -e --arg id "$viewer_id" \
	'any(.[]; .resource_type == "api_key" and .resource_id == $id and .action == "revoke")' >/dev/null

echo "honey scoped API keys smoke: ok"
