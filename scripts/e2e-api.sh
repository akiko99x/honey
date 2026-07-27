#!/usr/bin/env bash
# API lifecycle smoke test. Runs against an already-started disposable master.
#
# Covers the control-plane / API half of the P0 lifecycle acceptance:
#   derived onboarding -> domain -> node -> inbound -> user -> subscription;
#   group-based access -> subscription formats;
#   enrollment token issue + revoke; credential rotation; subscription-token
#   rotation revoking the old link; quota/expiry cutoff and recovery; traffic
#   reset; disable -> 410; API responses do not expose runtime private keys.
#
# The network half (real client connect, traffic counters growing, quota cutoff
# cert-revoke blocking an agent and restart recovery) needs a live node
# and stays in the separate Linux integration runbook.
set -euo pipefail

BASE="${HONEY_BASE_URL:-http://127.0.0.1:8080}"
TOKEN="${HONEY_API_TOKEN:?set HONEY_API_TOKEN}"
AUTH=(-H "authorization: Bearer ${TOKEN}" -H 'content-type: application/json')

need() { command -v "$1" >/dev/null || {
	echo "missing tool: $1" >&2
	exit 1
}; }
need curl
need jq

fail() {
	echo "smoke failed: $1" >&2
	exit 1
}

node_id=""
inbound_id=""
user_id=""
group_id=""
domain_id=""
cleanup() {
	[[ -z "$user_id" ]] || curl -fsS -X DELETE "${BASE}/users/${user_id}" "${AUTH[@]}" >/dev/null || true
	[[ -z "$inbound_id" ]] || curl -fsS -X DELETE "${BASE}/inbounds/${inbound_id}" "${AUTH[@]}" >/dev/null || true
	[[ -z "$node_id" ]] || curl -fsS -X DELETE "${BASE}/nodes/${node_id}" "${AUTH[@]}" >/dev/null || true
	[[ -z "$group_id" ]] || curl -fsS -X DELETE "${BASE}/groups/${group_id}" "${AUTH[@]}" >/dev/null || true
	[[ -z "$domain_id" ]] || curl -fsS -X DELETE "${BASE}/domains/${domain_id}" "${AUTH[@]}" >/dev/null || true
}
trap cleanup EXIT

suffix="$(date +%s)-$$"

onboarding_has() {
	local key="$1"
	curl -fsS "${BASE}/onboarding" "${AUTH[@]}" \
		| jq -e --arg key "$key" 'any(.steps[]; .key == $key and .complete == true)' >/dev/null
}

# --- first-run progress is derived from real resources ---------------------
curl -fsS "${BASE}/onboarding" "${AUTH[@]}" \
	| jq -e '[.steps[].key] == ["domain","node","inbound","user","subscription"]' >/dev/null \
	|| fail "onboarding steps or order"
domain_payload="$(jq -nc --arg host "smoke-${suffix}.invalid" '{host:$host, notes:"disposable onboarding smoke"}')"
domain="$(curl -fsS "${BASE}/domains" "${AUTH[@]}" -d "$domain_payload")"
domain_id="$(jq -r .id <<<"$domain")"
onboarding_has domain || fail "domain did not advance onboarding"

# --- create node / inbound / user (ungrouped node is universally accessible) -
node="$(curl -fsS "${BASE}/nodes" "${AUTH[@]}" -d "{\"name\":\"smoke-${suffix}\",\"address\":\"127.0.0.1\"}")"
node_id="$(jq -r .id <<<"$node")"
onboarding_has node || fail "node did not advance onboarding"
inbound="$(curl -fsS "${BASE}/inbounds" "${AUTH[@]}" -d "{\"node_id\":\"${node_id}\",\"tag\":\"smoke-in\",\"kind\":\"hysteria2\",\"listen_port\":18443,\"tls_enabled\":true,\"server_name\":\"example.com\",\"cert_path\":\"/etc/honey/fullchain.pem\",\"key_path\":\"/etc/honey/privkey.pem\"}")"
inbound_id="$(jq -r .id <<<"$inbound")"
onboarding_has inbound || fail "inbound did not advance onboarding"
jq -e 'has("reality_private_key") | not' <<<"$inbound" >/dev/null || fail "inbound API exposed private key field"
jq -e '.certificate_source == "manual" and .certificate_status == "configured"' <<<"$inbound" >/dev/null || fail "certificate status missing"
user="$(curl -fsS "${BASE}/users" "${AUTH[@]}" -d "{\"username\":\"smoke-${suffix}\",\"password\":\"smoke-secret\"}")"
user_id="$(jq -r .id <<<"$user")"
uuid_before="$(jq -r .uuid <<<"$user")"
sub_path="$(jq -r .subscription_path <<<"$user")"
revocable_sub_path="$(jq -r .revocable_subscription_path <<<"$user")"
onboarding_has user || fail "user did not advance onboarding"
onboarding_has subscription || fail "subscription did not advance onboarding"
curl -fsS "${BASE}/onboarding" "${AUTH[@]}" \
	| jq -e '.completed == .total and .total == 5' >/dev/null \
	|| fail "onboarding did not reach completion"

# --- labels are normalized metadata and do not alter entitlement/specs ------
curl -fsS -X PUT "${BASE}/nodes/${node_id}/labels" "${AUTH[@]}" -d '{"labels":[" Smoke ","region:test","smoke"]}' \
	| jq -e '.labels == ["region:test","smoke"]' >/dev/null || fail "node labels were not normalized"
curl -fsS -X PUT "${BASE}/inbounds/${inbound_id}/labels" "${AUTH[@]}" -d '{"labels":["protocol:hy2"]}' \
	| jq -e '.labels == ["protocol:hy2"]' >/dev/null || fail "inbound labels were not persisted"
curl -fsS -X PUT "${BASE}/users/${user_id}/labels" "${AUTH[@]}" -d '{"labels":["customer:smoke"]}' \
	| jq -e '.labels == ["customer:smoke"]' >/dev/null || fail "user labels were not persisted"
curl -fsS "${BASE}/labels?resource=issues" "${AUTH[@]}" \
	| jq -e 'index("region:test") != null and index("customer:smoke") != null' >/dev/null || fail "label catalog is incomplete"

# --- config preview is structural and never serializes credentials ----------
preview="$(curl -fsS "${BASE}/nodes/${node_id}/config-preview" "${AUTH[@]}")"
jq -e '.changed == true and .baseline_available == false and (.added | length) == 1' <<<"$preview" >/dev/null || fail "config preview missing candidate"
jq -e '[.. | objects | keys[]] | any(. == "uuid" or . == "password" or . == "private_key" or . == "extra_json") | not' <<<"$preview" >/dev/null || fail "config preview exposed sensitive fields"

# --- subscription documents render -----------------------------------------
curl -fsS -H "accept: application/json" "${BASE}${sub_path}" | jq -e '.links | length == 1' >/dev/null || fail "subscription links"
curl -fsS "${BASE}${sub_path}/sing-box" | jq -e '.outbounds | length >= 2' >/dev/null || fail "sing-box outbounds"

# --- group access removes and restores the endpoint deterministically --------
group="$(curl -fsS "${BASE}/groups" "${AUTH[@]}" -d "{\"name\":\"smoke-group-${suffix}\"}")"
group_id="$(jq -r .id <<<"$group")"
curl -fsS -X PUT "${BASE}/nodes/${node_id}/groups" "${AUTH[@]}" -d "{\"group_ids\":[\"${group_id}\"]}" >/dev/null
curl -fsS -H "accept: application/json" "${BASE}${sub_path}" | jq -e '.links | length == 0' >/dev/null || fail "group isolation did not hide endpoint"
curl -fsS -X PUT "${BASE}/users/${user_id}/groups" "${AUTH[@]}" -d "{\"group_ids\":[\"${group_id}\"]}" >/dev/null
curl -fsS -H "accept: application/json" "${BASE}${sub_path}" | jq -e '.links | length == 1' >/dev/null || fail "shared group did not restore endpoint"
curl -fsS -X PUT "${BASE}/users/${user_id}/groups" "${AUTH[@]}" -d '{"group_ids":[]}' >/dev/null
curl -fsS -H "accept: application/json" "${BASE}${sub_path}" | jq -e '.links | length == 0' >/dev/null || fail "removed user group still grants endpoint"
curl -fsS -X PUT "${BASE}/nodes/${node_id}/groups" "${AUTH[@]}" -d '{"group_ids":[]}' >/dev/null
curl -fsS -H "accept: application/json" "${BASE}${sub_path}" | jq -e '.links | length == 1' >/dev/null || fail "ungrouped node was not universal"
curl -fsS -X DELETE "${BASE}/groups/${group_id}" "${AUTH[@]}" >/dev/null
group_id=""

# --- enrollment token: issue, list active, revoke, list revoked ------------
enroll="$(curl -fsS -X POST "${BASE}/nodes/${node_id}/enrollments" "${AUTH[@]}" -d '{"expires_in_minutes":30}')"
enroll_id="$(jq -r .id <<<"$enroll")"
[[ -n "$(jq -r .token <<<"$enroll")" ]] || fail "enrollment token missing"
curl -fsS "${BASE}/nodes/${node_id}/enrollments" "${AUTH[@]}" \
	| jq -e --arg id "$enroll_id" 'any(.[]; .id == $id and .revoked_at == null)' >/dev/null \
	|| fail "issued enrollment not listed as active"
curl -fsS -X POST "${BASE}/enrollments/${enroll_id}/revoke" "${AUTH[@]}" >/dev/null
curl -fsS "${BASE}/nodes/${node_id}/enrollments" "${AUTH[@]}" \
	| jq -e --arg id "$enroll_id" 'any(.[]; .id == $id and .revoked_at != null)' >/dev/null \
	|| fail "revoked enrollment still active"

# --- credential rotation changes the uuid ----------------------------------
uuid_after="$(curl -fsS -X POST "${BASE}/users/${user_id}/rotate" "${AUTH[@]}" -d '{}' | jq -r .uuid)"
[[ "$uuid_after" != "$uuid_before" && -n "$uuid_after" ]] || fail "credential rotation did not change uuid"

# --- optional token rotation revokes only the old revocable link -----------
old_revocable_sub_path="$revocable_sub_path"
revocable_sub_path="$(curl -fsS -X POST "${BASE}/users/${user_id}/rotate-sub" "${AUTH[@]}" | jq -r .subscription_path)"
[[ "$revocable_sub_path" != "$old_revocable_sub_path" && -n "$revocable_sub_path" ]] || fail "revocable subscription rotation did not change the path"
old_status="$(curl -sS -o /dev/null -w '%{http_code}' "${BASE}${old_revocable_sub_path}")"
[[ "$old_status" == 404 ]] || fail "rotated-away revocable subscription should 404, got ${old_status}"
curl -fsS -H "accept: application/json" "${BASE}${sub_path}" | jq -e '.links | length == 1' >/dev/null || fail "permanent subscription broke after optional token rotation"
curl -fsS -H "accept: application/json" "${BASE}${revocable_sub_path}" | jq -e '.links | length == 1' >/dev/null || fail "new revocable subscription broken after rotation"

# --- periodic quota setting is persisted -----------------------------------
curl -fsS -X PUT "${BASE}/users/${user_id}/quota-interval" "${AUTH[@]}" -d '{"interval":"daily"}' >/dev/null
curl -fsS "${BASE}/users/${user_id}" "${AUTH[@]}" | jq -e '.quota_interval == "daily" and .quota_reset_at != null' >/dev/null || fail "daily quota interval not persisted"

# --- quota suppression/recovery (when the disposable DB is available) ------
if [[ -n "${DATABASE_URL:-}" ]] && command -v psql >/dev/null 2>&1; then
	curl -fsS -X PATCH "${BASE}/users/${user_id}" "${AUTH[@]}" -d '{"traffic_limit_bytes":1}' >/dev/null
	psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -c "UPDATE users SET used_traffic_bytes = 2 WHERE id = '${user_id}'" >/dev/null
	curl -fsS "${BASE}/users/${user_id}" "${AUTH[@]}" | jq -e '.active == false and .suppressed_reason == "quota"' >/dev/null || fail "quota did not suppress user"
	status="$(curl -sS -o /dev/null -w '%{http_code}' "${BASE}${sub_path}/v2ray")"
	[[ "$status" == 410 ]] || fail "quota-suppressed subscription should 410, got ${status}"
	curl -fsS -X POST "${BASE}/users/${user_id}/reset-traffic" "${AUTH[@]}" | jq -e '.active == true and (.used_traffic_bytes | tonumber) == 0' >/dev/null || fail "traffic reset did not recover quota"
	curl -fsS -X PATCH "${BASE}/users/${user_id}" "${AUTH[@]}" -d '{"traffic_limit_bytes":0}' >/dev/null
fi

# --- expiry suppression and null-patch recovery -----------------------------
curl -fsS -X PATCH "${BASE}/users/${user_id}" "${AUTH[@]}" -d '{"expires_at":"2000-01-01T00:00:00Z"}' | jq -e '.active == false and .suppressed_reason == "expired"' >/dev/null || fail "expiry did not suppress user"
status="$(curl -sS -o /dev/null -w '%{http_code}' "${BASE}${sub_path}/v2ray")"
[[ "$status" == 410 ]] || fail "expired subscription should 410, got ${status}"
curl -fsS -X PATCH "${BASE}/users/${user_id}" "${AUTH[@]}" -d '{"expires_at":null}' | jq -e '.active == true and .expires_at == null' >/dev/null || fail "null expiry patch did not recover user"
curl -fsS -H "accept: application/json" "${BASE}${sub_path}" | jq -e '.status == "active"' >/dev/null || fail "subscription did not recover after expiry removal"

# --- traffic reset zeroes the counter --------------------------------------
curl -fsS -X POST "${BASE}/users/${user_id}/reset-traffic" "${AUTH[@]}" \
	| jq -e '(.used_traffic_bytes | tonumber) == 0' >/dev/null || fail "traffic reset did not zero counter"

# --- disable makes the subscription 410 ------------------------------------
curl -fsS -X PATCH "${BASE}/users/${user_id}" "${AUTH[@]}" -d '{"enabled":false}' \
	| jq -e '.active == false' >/dev/null || fail "user did not disable"
status="$(curl -sS -o /dev/null -w '%{http_code}' "${BASE}${sub_path}/v2ray")"
[[ "$status" == 410 ]] || fail "disabled subscription should 410, got ${status}"
curl -fsS "${BASE}/issues" "${AUTH[@]}" \
	| jq -e --arg id "$user_id" 'any(.issues[]; .entity_id == $id and (.labels | index("customer:smoke")) != null)' >/dev/null \
	|| fail "user issue did not inherit labels"

echo "honey api smoke: ok"
