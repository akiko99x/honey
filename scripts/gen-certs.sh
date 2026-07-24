#!/usr/bin/env bash
# Creates one CA/master identity plus a unique server certificate per node.
# usage: ./scripts/gen-certs.sh <node-id> <agent-ip-or-dns> [pki-dir]
set -euo pipefail

NODE_ID="${1:?usage: gen-certs.sh <node-id> <agent-ip-or-dns> [pki-dir]}"
AGENT_HOST="${2:?usage: gen-certs.sh <node-id> <agent-ip-or-dns> [pki-dir]}"
PKI="${3:-./certs}"
DAYS="${HONEY_CERT_DAYS:-825}"

[[ "$NODE_ID" =~ ^[A-Za-z0-9-]+$ ]] || {
	echo "node-id may contain letters, digits and dashes only" >&2
	exit 1
}
TLS_NAME="node-${NODE_ID}.honey"
mkdir -p "$PKI"
PKI="$(cd "$PKI" && pwd)"
NODE_OUT="${PKI}/nodes/${NODE_ID}"
mkdir -p "$NODE_OUT"

if [[ ! -f "${PKI}/ca.key" ]]; then
	openssl genrsa -out "${PKI}/ca.key" 4096
	openssl req -x509 -new -nodes -key "${PKI}/ca.key" -sha256 -days "$DAYS" \
		-subj "/O=honey/CN=honey-ca" -out "${PKI}/ca.crt"
	chmod 0600 "${PKI}/ca.key"
	echo "[ca] created"
fi

sign_cert() {
	local name="$1" cn="$2" ext="$3" out="$4"
	openssl genrsa -out "${out}/${name}.key" 2048
	openssl req -new -key "${out}/${name}.key" -subj "/O=honey/CN=${cn}" -out "${out}/${name}.csr"
	openssl x509 -req -in "${out}/${name}.csr" -CA "${PKI}/ca.crt" -CAkey "${PKI}/ca.key" \
		-CAcreateserial -days "$DAYS" -sha256 -extfile "$ext" -out "${out}/${name}.crt"
	rm -f "${out}/${name}.csr"
	chmod 0600 "${out}/${name}.key"
}

if [[ ! -f "${PKI}/master.crt" ]]; then
	master_ext="$(mktemp)"
	printf '%s\n' 'extendedKeyUsage = clientAuth' >"$master_ext"
	sign_cert master honey-master "$master_ext" "$PKI"
	rm -f "$master_ext"
	echo "[cert] master.crt"
fi

if [[ "$AGENT_HOST" == *:* || "$AGENT_HOST" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
	host_san="IP:${AGENT_HOST}"
else
	host_san="DNS:${AGENT_HOST}"
fi
agent_ext="$(mktemp)"
printf 'subjectAltName = DNS:%s,%s\nextendedKeyUsage = serverAuth\n' "$TLS_NAME" "$host_san" >"$agent_ext"
sign_cert agent "$TLS_NAME" "$agent_ext" "$NODE_OUT"
rm -f "$agent_ext"
cp "${PKI}/ca.crt" "${NODE_OUT}/ca.crt"

cat <<EOF
[cert] ${NODE_OUT}/agent.crt
done.
  node files      : ${NODE_OUT}/{ca.crt,agent.crt,agent.key}
  master files    : ${PKI}/{ca.crt,master.crt,master.key}
  tls_server_name : ${TLS_NAME}
set this value on the node record before connecting.
EOF
