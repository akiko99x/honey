#!/usr/bin/env bash
# Disposable Linux master+agent lifecycle rehearsal.
#
# Uses an isolated PostgreSQL database, temporary PKI/config directories and
# fake core executables that implement only version/check/run. It exercises the
# real HTTP API, enrollment CSR flow, mTLS gRPC channel, config builders,
# agent-side dry-run, apply markers, certificate revoke/re-enrollment, and
# offline restart recovery. It never touches /etc/honey, systemd, firewall
# rules, or production core processes.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/honey-lifecycle.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
MASTER_BIN="${HONEY_MASTER_BIN:-$ROOT/master/target/debug/honey-master}"
AGENT_BIN="${HONEY_AGENT_BIN:-$WORK/bin/honey-agent}"
ENROLL_BIN="${HONEY_ENROLL_BIN:-$WORK/bin/honey-enroll}"
ADMIN_DATABASE_URL="${ADMIN_DATABASE_URL:?set ADMIN_DATABASE_URL to a maintenance database}"
API_TOKEN="${HONEY_API_TOKEN:-lifecycle-test-token}"
SECRET_KEY="${HONEY_SECRET_KEY:-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=}"
DB_NAME="honey_lifecycle_$(date +%s)_$$"
DB_BASE="${ADMIN_DATABASE_URL%/*}"
DATABASE_URL="${DB_BASE}/${DB_NAME}"
PORT_BASE=$((20000 + ($$ % 10000)))
API_PORT="${HONEY_LIFECYCLE_API_PORT:-$PORT_BASE}"
AGENT_PORT="${HONEY_LIFECYCLE_AGENT_PORT:-$((PORT_BASE + 1))}"
INBOUND_PORT="${HONEY_LIFECYCLE_INBOUND_PORT:-$((PORT_BASE + 2))}"
BASE="http://127.0.0.1:${API_PORT}"
AUTH=(-H "authorization: Bearer ${API_TOKEN}" -H 'content-type: application/json')
master_pid=""
agent_pid=""
node_id=""
inbound_id=""
user_id=""

need() { command -v "$1" >/dev/null || { echo "missing tool: $1" >&2; exit 1; }; }
for tool in curl jq openssl psql setsid; do need "$tool"; done

stop_pid() {
	local pid="${1:-}"
	[[ -z "$pid" ]] && return 0
	kill "$pid" 2>/dev/null || true
	for _ in $(seq 1 30); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.1; done
	kill -KILL "$pid" 2>/dev/null || true
}

stop_agent_group() {
	[[ -z "$agent_pid" ]] && return 0
	# setsid makes the agent and its fake core children one disposable group.
	kill -TERM -- "-$agent_pid" 2>/dev/null || stop_pid "$agent_pid"
	for _ in $(seq 1 50); do kill -0 "$agent_pid" 2>/dev/null || break; sleep 0.1; done
	agent_pid=""
}

cleanup() {
	set +e
	stop_agent_group
	stop_pid "$master_pid"
	psql "$ADMIN_DATABASE_URL" -v ON_ERROR_STOP=1 -v db="$DB_NAME" <<'SQL' >/dev/null 2>&1
SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = :'db';
DROP DATABASE IF EXISTS :"db";
SQL
	rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

[[ "$(uname -s)" == "Linux" ]] || { echo "lifecycle harness requires Linux" >&2; exit 1; }
for port in "$API_PORT" "$AGENT_PORT" "$INBOUND_PORT"; do
	if command -v ss >/dev/null && ss -H -ltn "sport = :$port" | grep -q .; then
		echo "port $port is occupied" >&2
		exit 1
	fi
done

if [[ ! -x "$MASTER_BIN" ]]; then
	command -v cargo >/dev/null || { echo "missing master binary and cargo" >&2; exit 1; }
	cargo build --locked --manifest-path "$ROOT/master/Cargo.toml"
fi
if [[ ! -x "$AGENT_BIN" || ! -x "$ENROLL_BIN" ]]; then
	need go
	mkdir -p "$(dirname "$AGENT_BIN")" "$(dirname "$ENROLL_BIN")"
	(cd "$ROOT/agent" && go build -o "$AGENT_BIN" ./cmd/agent && go build -o "$ENROLL_BIN" ./cmd/enroll)
fi

echo "[lifecycle] creating disposable database $DB_NAME"
psql "$ADMIN_DATABASE_URL" -v ON_ERROR_STOP=1 -v db="$DB_NAME" -c 'CREATE DATABASE :"db"' >/dev/null
DATABASE_URL="$DATABASE_URL" HONEY_SECRET_KEY="$SECRET_KEY" "$MASTER_BIN" migrate >/dev/null

mkdir -p "$WORK/pki" "$WORK/agent-certs" "$WORK/config"
HONEY_CERT_DAYS=2 bash "$ROOT/scripts/gen-certs.sh" bootstrap 127.0.0.1 "$WORK/pki" >/dev/null

start_master() {
	DATABASE_URL="$DATABASE_URL" HONEY_SECRET_KEY="$SECRET_KEY" HONEY_API_TOKEN="$API_TOKEN" \
		"$MASTER_BIN" serve --listen "127.0.0.1:${API_PORT}" --certs-dir "$WORK/pki" \
		>"$WORK/master.log" 2>&1 &
	master_pid=$!
	for _ in $(seq 1 60); do
		curl -fsS "$BASE/ready" >/dev/null 2>&1 && return 0
		kill -0 "$master_pid" 2>/dev/null || { tail -n 80 "$WORK/master.log" >&2; return 1; }
		sleep 0.25
	done
	echo "master did not become ready" >&2
	return 1
}

start_agent() {
	setsid "$AGENT_BIN" \
		--mode serve --listen "127.0.0.1:${AGENT_PORT}" --node-id "$node_id" \
		--ca "$WORK/agent-certs/ca.crt" --cert "$WORK/agent-certs/agent.crt" --key "$WORK/agent-certs/agent.key" \
		--singbox-bin "$WORK/fake-core" --singbox-config "$WORK/config/sing-box.json" \
		--xray-bin "$WORK/fake-core" --xray-config "$WORK/config/xray.json" \
		--clash-url "http://127.0.0.1:1" --xray-api "127.0.0.1:1" \
		>>"$WORK/agent.log" 2>&1 &
	agent_pid=$!
	for _ in $(seq 1 60); do
		curl -fsS "$BASE/nodes/$node_id/logs?limit=5" "${AUTH[@]}" >/dev/null 2>&1 && return 0
		kill -0 "$agent_pid" 2>/dev/null || { tail -n 80 "$WORK/agent.log" >&2; return 1; }
		sleep 0.25
	done
	echo "agent did not become reachable" >&2
	return 1
}

printf '%s\n' \
	'#!/usr/bin/env bash' \
	'set -euo pipefail' \
	'case "${1:-}" in' \
	'  version) if [[ "$(basename "$0")" == *xray* ]]; then echo "Xray 0.0.0-test"; else echo "sing-box version 0.0.0-test"; fi; exit 0 ;;' \
	'  check) exit 0 ;;' \
	'  run) for arg in "$@"; do [[ "$arg" == "-test" ]] && exit 0; done; trap "exit 0" TERM INT; while :; do sleep 1; done ;;' \
	'  *) exit 0 ;;' \
	'esac' >"$WORK/fake-core"
chmod 0755 "$WORK/fake-core"

start_master
echo "[lifecycle] create node and perform one-time CSR enrollment"
node_id="$(curl -fsS "$BASE/nodes" "${AUTH[@]}" \
	-d "{\"name\":\"lifecycle-$$\",\"address\":\"127.0.0.1\",\"grpc_port\":${AGENT_PORT},\"transport\":\"serve\"}" | jq -r .id)"
enrollment="$(curl -fsS -X POST "$BASE/nodes/$node_id/enrollments" "${AUTH[@]}" -d '{"expires_in_minutes":10}')"
enrollment_token="$(jq -r .token <<<"$enrollment")"
"$ENROLL_BIN" --master "$BASE" --token "$enrollment_token" --certs-dir "$WORK/agent-certs" \
	--env-file "$WORK/agent.env" --listen "127.0.0.1:${AGENT_PORT}" >/dev/null
[[ -s "$WORK/agent-certs/agent.key" && -s "$WORK/agent-certs/agent.crt" ]] || { echo "enrollment did not write certificates" >&2; exit 1; }

start_agent
echo "[lifecycle] create candidate, preview it, and validate without applying"
user_id="$(curl -fsS "$BASE/users" "${AUTH[@]}" -d "{\"username\":\"lifecycle-$$\",\"password\":\"temporary-test-secret\"}" | jq -r .id)"
inbound_id="$(curl -fsS "$BASE/inbounds" "${AUTH[@]}" \
	-d "{\"node_id\":\"${node_id}\",\"tag\":\"lifecycle-in\",\"kind\":\"vless\",\"core\":\"xray\",\"listen\":\"127.0.0.1\",\"listen_port\":${INBOUND_PORT}}" | jq -r .id)"
curl -fsS "$BASE/nodes/$node_id/config-preview" "${AUTH[@]}" \
	| jq -e '.changed == true and .baseline_available == false and (.added | length) == 1 and .restart_cores == ["xray"]' >/dev/null
[[ ! -e "$WORK/config/xray.json" ]] || { echo "candidate existed before dry-run" >&2; exit 1; }
curl -fsS -X POST "$BASE/nodes/$node_id/dry-run" "${AUTH[@]}" -d '{}' \
	| jq -e '.state != "Errored" and (.message | contains("no changes applied"))' >/dev/null
[[ ! -e "$WORK/config/xray.json" && ! -e "$WORK/config/xray.json.honey-state.json" ]] \
	|| { echo "dry-run mutated live agent state" >&2; exit 1; }

echo "[lifecycle] apply and persist a recovery-authorized config"
curl -fsS -X POST "$BASE/nodes/$node_id/push" "${AUTH[@]}" -d '{}' | jq -e '.state == "Running"' >/dev/null
[[ -s "$WORK/config/xray.json" && -s "$WORK/config/xray.json.honey-state.json" ]] || { echo "apply did not persist config and marker" >&2; exit 1; }
jq -e '.version == 1 and .active == true and (.sha256 | length == 64)' "$WORK/config/xray.json.honey-state.json" >/dev/null
curl -fsS "$BASE/nodes/$node_id/config-preview" "${AUTH[@]}" \
	| jq -e '.changed == false and .baseline_available == true and (.added + .removed + .modified | length) == 0' >/dev/null

echo "[lifecycle] restart agent while master is offline and verify local recovery"
stop_pid "$master_pid"
master_pid=""
stop_agent_group
setsid "$AGENT_BIN" \
	--mode serve --listen "127.0.0.1:${AGENT_PORT}" --node-id "$node_id" \
	--ca "$WORK/agent-certs/ca.crt" --cert "$WORK/agent-certs/agent.crt" --key "$WORK/agent-certs/agent.key" \
	--singbox-bin "$WORK/fake-core" --singbox-config "$WORK/config/sing-box.json" \
	--xray-bin "$WORK/fake-core" --xray-config "$WORK/config/xray.json" \
	--clash-url "http://127.0.0.1:1" --xray-api "127.0.0.1:1" >>"$WORK/agent.log" 2>&1 &
agent_pid=$!
for _ in $(seq 1 60); do
	grep -q 'resumed xray from last config' "$WORK/agent.log" && break
	kill -0 "$agent_pid" 2>/dev/null || { tail -n 80 "$WORK/agent.log" >&2; exit 1; }
	sleep 0.25
done
grep -q 'resumed xray from last config' "$WORK/agent.log" || { echo "offline recovery was not observed" >&2; exit 1; }

start_master
for _ in $(seq 1 60); do
	curl -fsS "$BASE/nodes/$node_id/logs?limit=20" "${AUTH[@]}" | jq -e 'length > 0' >/dev/null 2>&1 && break
	sleep 0.25
done
curl -fsS "$BASE/nodes/$node_id/logs?limit=20" "${AUTH[@]}" | jq -e 'length > 0' >/dev/null

echo "[lifecycle] revoke the presented certificate and reject reconnect"
certificate_id="$(curl -fsS "$BASE/nodes/$node_id/certificates" "${AUTH[@]}" | jq -r '[.[] | select(.revoked_at == null)][0].id')"
[[ -n "$certificate_id" && "$certificate_id" != "null" ]] || { echo "active certificate missing" >&2; exit 1; }
curl -fsS -X POST "$BASE/certificates/$certificate_id/revoke" "${AUTH[@]}" -d '{}' >/dev/null
rejected_status="$(curl -sS -o "$WORK/revoked-response.json" -w '%{http_code}' "$BASE/nodes/$node_id/logs?limit=5" "${AUTH[@]}")"
[[ "$rejected_status" == "502" ]] || { echo "revoked certificate reconnect returned $rejected_status" >&2; exit 1; }

echo "[lifecycle] issue a replacement identity and recover the node"
replacement="$(curl -fsS -X POST "$BASE/nodes/$node_id/enrollments" "${AUTH[@]}" -d '{"expires_in_minutes":10}')"
replacement_token="$(jq -r .token <<<"$replacement")"
stop_agent_group
"$ENROLL_BIN" --master "$BASE" --token "$replacement_token" --certs-dir "$WORK/agent-certs" \
	--env-file "$WORK/agent.env" --listen "127.0.0.1:${AGENT_PORT}" --force >/dev/null
start_agent
curl -fsS "$BASE/nodes/$node_id/certificates" "${AUTH[@]}" \
	| jq -e '([.[] | select(.revoked_at != null)] | length) == 1 and ([.[] | select(.revoked_at == null)] | length) == 1' >/dev/null

curl -fsS -X DELETE "$BASE/users/$user_id" "${AUTH[@]}" >/dev/null; user_id=""
curl -fsS -X DELETE "$BASE/inbounds/$inbound_id" "${AUTH[@]}" >/dev/null; inbound_id=""
curl -fsS -X DELETE "$BASE/nodes/$node_id" "${AUTH[@]}" >/dev/null; node_id=""
echo "honey disposable linux lifecycle: ok"
