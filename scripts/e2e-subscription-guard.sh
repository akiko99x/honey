#!/usr/bin/env bash
# Public subscription guard smoke for an already-started disposable master.
# It temporarily lowers the live request budget and deliberately produces a
# rate-limit event. Never point it at production.
set -euo pipefail

BASE="${HONEY_BASE_URL:-http://127.0.0.1:8080}"
SUB_URL="${HONEY_SUB_URL:?set HONEY_SUB_URL to a disposable active subscription URL}"
USERNAME="${HONEY_ADMIN_USERNAME:?set HONEY_ADMIN_USERNAME}"
PASSWORD="${HONEY_ADMIN_PASSWORD:?set HONEY_ADMIN_PASSWORD}"
TOTP="${HONEY_ADMIN_TOTP:-}"

for tool in curl jq mktemp; do
	command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done

jar="$(mktemp)"
headers="$(mktemp)"
body_file="$(mktemp)"
original=""
authenticated=false
cleanup() {
	if [[ "$authenticated" == true && -n "$original" ]]; then
		restore="$(jq -c '{
			subscription_guard_enabled,
			subscription_guard_max_requests,
			subscription_guard_window_secs,
			subscription_guard_block_secs
		}' <<<"$original")"
		curl -fsS -b "$jar" -H 'content-type: application/json' -X PATCH \
			-d "$restore" "${BASE}/settings" >/dev/null || true
	fi
	rm -f "$jar" "$headers" "$body_file"
}
trap cleanup EXIT

login="$(jq -cn --arg username "$USERNAME" --arg password "$PASSWORD" --arg totp_code "$TOTP" \
	'{username:$username,password:$password} + (if $totp_code == "" then {} else {totp_code:$totp_code} end)')"
curl -fsS -c "$jar" -b "$jar" -H 'content-type: application/json' \
	-d "$login" "${BASE}/auth/login" >/dev/null
authenticated=true

original="$(curl -fsS -b "$jar" "${BASE}/settings")"
test_settings='{"subscription_guard_enabled":true,"subscription_guard_max_requests":10,"subscription_guard_window_secs":60,"subscription_guard_block_secs":10}'
curl -fsS -b "$jar" -H 'content-type: application/json' -X PATCH \
	-d "$test_settings" "${BASE}/settings" | jq -e \
	'.subscription_guard_enabled and .subscription_guard_max_requests == 10' >/dev/null

for _ in $(seq 1 10); do
	status="$(curl -sS -o /dev/null -w '%{http_code}' "$SUB_URL")"
	[[ "$status" == 200 ]] || { echo "expected subscription HTTP 200, got $status" >&2; exit 1; }
done

status="$(curl -sS -D "$headers" -o "$body_file" -w '%{http_code}' "$SUB_URL")"
[[ "$status" == 429 ]] || { echo "expected HTTP 429 after budget, got $status" >&2; exit 1; }
jq -e '.code == "M1701" and .retry_after >= 1' "$body_file" >/dev/null
grep -Eiq '^retry-after: [1-9][0-9]*' "$headers"
grep -Eiq '^cache-control: private, no-store, max-age=0' "$headers"
grep -Eiq '^referrer-policy: no-referrer' "$headers"
grep -Eiq '^x-content-type-options: nosniff' "$headers"
if grep -Fq "$SUB_URL" "$body_file"; then
	echo "rate-limit response exposed the subscription URL" >&2
	exit 1
fi

curl -fsS -b "$jar" "${BASE}/settings" |
	jq -e '.subscription_guard_blocked_total >= 1' >/dev/null
sleep 1
curl -fsS -b "$jar" "${BASE}/notifications?event=subscription_abuse&limit=20" |
	jq -e 'any(.[]; .code == "M1701" and .severity == "warning")' >/dev/null

echo "honey public subscription guard smoke: ok"
