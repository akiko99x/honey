#!/usr/bin/env bash
set -euo pipefail

if [[ -r /etc/honey/agent.env ]]; then
	set -a
	# shellcheck disable=SC1091
	. /etc/honey/agent.env
	set +a
fi

if (($# > 0)); then
	exec "$@"
fi

exec /usr/local/bin/honey-agent \
	--mode "${HONEY_MODE:-serve}" \
	--listen "${HONEY_LISTEN:-0.0.0.0:8443}" \
	--master-addr "${HONEY_MASTER_ADDR:-127.0.0.1:9443}" \
	--ca /etc/honey/certs/ca.crt \
	--cert /etc/honey/certs/agent.crt \
	--key /etc/honey/certs/agent.key \
	--node-id "${HONEY_NODE_ID:-node-1}" \
	--singbox-bin /usr/local/bin/sing-box \
	--singbox-config /etc/honey/sing-box/config.json \
	--clash-url "${HONEY_CLASH_URL:-http://127.0.0.1:9090}" \
	--clash-secret "${HONEY_CLASH_SECRET:-}" \
	--xray-bin /usr/local/bin/xray \
	--xray-config /etc/honey/xray/config.json \
	--xray-api "${HONEY_XRAY_API:-127.0.0.1:8081}" \
	--xray-acme-root "${HONEY_XRAY_ACME_ROOT:-/etc/honey/xray/acme}" \
	--xray-acme-listen "${HONEY_XRAY_ACME_LISTEN:-127.0.0.1:9080}" \
	--singbox-acme-upstream "${HONEY_SINGBOX_ACME_UPSTREAM:-127.0.0.1:9082}"
