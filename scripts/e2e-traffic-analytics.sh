#!/usr/bin/env bash
set -euo pipefail

# Disposable smoke for traffic history/API scope. Run from a Linux checkout;
# callers provide a temporary DATABASE_URL and a running honey-master.
: "${MASTER_URL:?set MASTER_URL, e.g. http://127.0.0.1:8080}"
: "${HONEY_API_TOKEN:?set HONEY_API_TOKEN for the owner smoke identity}"

base="${MASTER_URL%/}"
auth=(-H "Authorization: Bearer ${HONEY_API_TOKEN}" -H 'accept: application/json')
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo '== health =='
curl -fsS "${auth[@]}" "$base/health" | jq -e '.status == "ok"' >/dev/null

echo '== bounded analytics =='
curl -fsS "${auth[@]}" "$base/analytics/traffic?bucket=hour" >"$tmp/analytics.json"
jq -e '(.scope == "fleet") and (.bucket == "hour") and (.summary.total_bytes >= 0) and (.series | type == "array")' "$tmp/analytics.json" >/dev/null

echo '== csv export =='
curl -fsS "${auth[@]}" "$base/analytics/traffic.csv?bucket=hour" -o "$tmp/traffic.csv"
test "$(head -n 1 "$tmp/traffic.csv")" = 'bucket,upload_bytes,download_bytes,total_bytes'

echo 'traffic analytics smoke passed'
