# example: one node, many protocols

sing-box runs **one process** with several inbounds, so a single node serves
vless (reality) and hysteria2 (and more) at the same time. the master sends a
`NodeSpec`; the agent turns it into `config.json`, runs `sing-box check`, then
`sing-box run`.

## NodeSpec the master would send (shown as json for readability)

```json
{
  "clash_listen": "127.0.0.1:9090",
  "inbounds": [
    {
      "tag": "vless-in",
      "type": "vless",
      "listen_port": 443,
      "users": [
        { "name": "alice", "uuid": "11111111-1111-1111-1111-111111111111", "flow": "xtls-rprx-vision" }
      ],
      "tls": {
        "enabled": true,
        "server_name": "example.com",
        "reality": {
          "private_key": "<reality-private-key>",
          "short_ids": ["0123abcd"],
          "handshake_server": "example.com",
          "handshake_port": 443
        }
      }
    },
    {
      "tag": "hy2-in",
      "type": "hysteria2",
      "listen_port": 8443,
      "users": [{ "name": "alice", "password": "s3cret" }],
      "tls": {
        "enabled": true,
        "server_name": "example.com",
        "cert_path": "/etc/honey/tls/fullchain.pem",
        "key_path": "/etc/honey/tls/key.pem"
      },
      "extra_json": "{\"obfs\":{\"type\":\"salamander\",\"password\":\"o6fs\"}}"
    }
  ]
}
```

`extra_json` is merged verbatim into the inbound, so protocol-specific knobs
(hysteria2 obfs, transport, multiplex, tuic congestion control, ...) work
without changing the proto.

## config.json the agent generates

```json
{
  "log": { "level": "info", "timestamp": true },
  "experimental": {
    "clash_api": { "external_controller": "127.0.0.1:9090", "secret": "" }
  },
  "inbounds": [
    {
      "type": "vless",
      "tag": "vless-in",
      "listen": "::",
      "listen_port": 443,
      "users": [{ "name": "alice", "uuid": "1111...", "flow": "xtls-rprx-vision" }],
      "tls": {
        "enabled": true,
        "server_name": "example.com",
        "reality": {
          "enabled": true,
          "private_key": "<reality-private-key>",
          "handshake": { "server": "example.com", "server_port": 443 },
          "short_id": ["0123abcd"]
        }
      }
    },
    {
      "type": "hysteria2",
      "tag": "hy2-in",
      "listen": "::",
      "listen_port": 8443,
      "users": [{ "name": "alice", "password": "s3cret" }],
      "tls": {
        "enabled": true,
        "server_name": "example.com",
        "certificate_path": "/etc/honey/tls/fullchain.pem",
        "key_path": "/etc/honey/tls/key.pem"
      },
      "obfs": { "type": "salamander", "password": "o6fs" }
    }
  ],
  "outbounds": [{ "type": "direct", "tag": "direct" }]
}
```

the `experimental.clash_api` block is what the agent's `Stats` stream reads for
live traffic (`GET /connections`).
