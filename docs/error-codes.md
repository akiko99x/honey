# honey diagnostic codes

Honey emits short, stable diagnostic codes in logs and selected API responses.
This file is the operator catalog: what a code means, where it is emitted, and
what to do next.

The source of truth is the code in `agent/internal/logx/logx.go` and the
`code="M####"` fields in `master/src`. This document is deliberately reviewed
against those sources; a code that appears only in the legacy section is not a
current runtime event.

## 1. Reading a log record

Agent records use:

```text
<level> <code> <message>
```

The master uses structured tracing. In pretty output the code is rendered beside
the message; in JSON output it is the `code` field.

| Level | Meaning | Operator response |
|---|---|---|
| `debug` | expected/chattery lifecycle detail | use for correlation, not alerting |
| `info` | normal milestone or state transition | normally no action |
| `warn` | recoverable failure or attention needed | investigate if repeated or persistent |
| `error` | requested action failed | investigate immediately; check rollback/state |

Messages intentionally use loose lowercase English. Parse the code, level and
structured fields (`node_id`, `request_id`, `core`, `source`) instead of
matching prose.

Never ship raw core stdout, private keys, subscription tokens, bearer tokens or
database errors to a public log sink. The agent's authenticated log snapshot
redacts credential-shaped values before the master can store or display them.

## 2. Fast lookup by symptom

| Symptom | First codes | Check |
|---|---|---|
| agent never starts | `A0102`, `A0103`, `A0203` | mTLS files, mode/listen flags, port ownership |
| NAT node never registers | `A0201`, `M0407`, `M0409` | node → master dial address, CA/SAN, firewall |
| node is offline | `M0401`, `M0403`, `M0407`, `M0408`, `M0409`, `M0810` | direction, node ID, certificate inventory, listener |
| push failed | `M0406`, `N0102`, `N0103`, `N0112`, `N0113` | dry-run, core checker, rollback and disk permissions |
| core crashed | `N0106`, `N0109`, `N0110` | core stdout/journal, config and binary version |
| stats stopped | `N0301`, `N0303`, `M0601`, `M0612` | Clash API/Xray StatsService, database and retention |
| Hysteria2 port hopping failed | `N0304` | nft binary, permissions and `inet honey` table |
| certificate/enrollment issue | `M0801`–`M0811`, `M0901`–`M0904` | CA, SAN, expiry, revocation and ACME |
| panel/API denied | `M1002`, `M1003`, `M1207`, `M1209`, `M1210` | allowed host/path, auth, origin and role |
| subscription unavailable | `M0702`, `M0703`, `M0704`, `M1701`, `M1702` | token state, endpoint rendering and guard budget |
| domain/endpoint unreachable | `M1301`, `M1302`, `M1501`, `M1502` | DNS, certificate, TCP/UDP route and vantage |

## 3. Code families

- `A####` — the Go agent and `honey-enroll`.
- `N####` — a node-local core or managed service (sing-box/Xray/WireGuard/
  MTProto/NaiveProxy).
- `M####` — the Rust master, API, database and background monitors.

Codes are not HTTP status codes. API errors use the `M12xx` family and include
an HTTP status separately. A code is stable: do not reuse it for a different
meaning. Add new codes at the end of the relevant subsystem range.

## 4. `A####` — agent and enrollment

### A01xx — boot and resume

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `A0101` | info | agent process starts | confirm node ID and binary version |
| `A0102` | error | mTLS server config cannot load | check CA/cert/key paths and permissions |
| `A0103` | error | transport wiring fails | check `HONEY_MODE`, addresses and flags |
| `A0104` | info | a transport comes up | no action |
| `A0105` | warn | a transport stops | inspect the transport error and restart policy |
| `A0106` | info | graceful shutdown begins | no action |
| `A0107` | info | a core resumes from an active persisted config | verify the resumed core is expected |
| `A0108` | warn | resume attempt fails | inspect saved config, marker and core journal |
| `A0109` | info | no active saved config exists | wait for the first master push |
| `A0110` | warn | persisted config path cannot be read | check disk path and service-account permissions |

### A02xx — dial transport

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `A0201` | warn | dial to master fails and will retry | check DNS, TCP `9443`, CA and backoff |
| `A0202` | info | outbound dial tunnel is established | no action |
| `A0203` | error | serve listener cannot bind | resolve port conflict or bind permission |

### A03xx — authenticated RPC

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `A0301` | debug | master asks `WhoRU` | no action |
| `A0302` | info | `Apply` arrives | correlate with the node push |
| `A0303` | info | `Start` arrives | no action |
| `A0304` | info | `Stop` arrives | no action |
| `A0305` | debug | stats/connections stream is requested | no action |
| `A0306` | warn | RPC names an unknown core | fix master/agent core capability mismatch |
| `A0307` | debug | master requests a finite log snapshot | no action |

### A04xx — one-time enrollment client

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `A0401` | info | `honey-enroll` starts | no action |
| `A0402` | info | local keypair and CSR are generated | verify the private key stays on the node |
| `A0403` | error | master rejects the claim | issue a fresh token and check node ownership |
| `A0404` | error | master response is incomplete/invalid | inspect HTTPS endpoint and response body |
| `A0405` | info | certificates are written successfully | start the agent |
| `A0499` | error | another enrollment failure occurs | inspect the complete local error and retry safely |

## 5. `N####` — core and node-local services

### N01xx — config and process lifecycle

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `N0101` | debug | config is built from the node spec | no action |
| `N0102` | error | config builder rejects the spec | fix protocol/core/field validation |
| `N0103` | error | the core's own config check rejects a candidate | run `sing-box check`/`xray -test` and read stderr |
| `N0104` | info | core start is requested | no action |
| `N0105` | info | core starts with a PID | verify the listener if the client still fails |
| `N0106` | error | core cannot start | inspect binary, config, port and permissions |
| `N0107` | info | graceful core stop begins | no action |
| `N0108` | info | core stops cleanly | no action |
| `N0109` | warn | core exits unexpectedly | inspect core journal and restart count |
| `N0110` | warn | core is hard-killed after grace period | investigate hangs or corrupted process state |
| `N0111` | info | validated config is being swapped | no action |
| `N0112` | warn | apply failed and previous config was restored | inspect the candidate error; service should remain usable |
| `N0113` | error | apply and rollback/restart of previous config both failed | stop automatic changes and recover the previous config manually |
| `N0114` | warn | the new config file cannot be installed | check `/etc/honey`, ownership and disk space |

### N02xx — core versions

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `N0201` | debug | a core reports its version | no action |
| `N0202` | warn | a version query fails | check executable path and `--version` support |

### N03xx — stats, firewall, quota and managed services

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `N0301` | warn | sing-box Clash API is unreachable and stats pause | check `HONEY_CLASH_URL` and the sing-box process |
| `N0302` | info | sing-box Clash API recovers | no action |
| `N0303` | warn | Xray StatsService query fails | check Xray API address and stats config |
| `N0304` | warn | Hysteria2 port-hopping nft setup fails | check nftables binary/table permissions |
| `N0305` | info/warn | WireGuard apply succeeds or fails | inspect WireGuard capability and interface state |
| `N0306` | info/warn | local quota cuts connections or persistence fails | inspect quota state and the local accounting directory |
| `N0307` | info/warn | managed MTProto/Naive services start or apply fails | check the external daemon binary and generated config |

## 6. `M####` — master runtime

### M01xx — service boot and background loops

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0101` | info | master service starts | no action |
| `M0102` | info | plain HTTP REST API is listening | keep bind loopback-only or put HTTPS in front |
| `M0103` | info | built-in HTTPS is listening | verify certificate and host |
| `M0104` | info | built-in HTTPS + ACME is listening | verify DNS/TLS-ALPN reachability |
| `M0106` | info | reconcile loop starts | no action |
| `M0107` | info | stats collector starts | no action |
| `M0108` | warn | a background service task exits unexpectedly | inspect the task-specific code |
| `M0109` | error | a background task returns an error | inspect the task-specific code and restart policy |
| `M0110` | error | a background task panics | capture the journal and restart safely |
| `M0111` | error | public unauthenticated API bind is refused | bind loopback or configure an authenticated mode |
| `M0112` | error | no secret key is available for secret operations | set a valid key backend before starting |
| `M0113` | info/warn | uptime sampler starts or sample/prune fails | inspect database availability if warnings repeat |
| `M0114` | info | at-rest encryption key backend is selected | confirm the backend is the intended one |
| `M0115` | info/warn | GeoIP table loads or cannot be read | fix the optional file if geography is required |
| `M0119` | info | HA lease election starts | no action |
| `M0120` | info | this instance becomes HA leader | no action |
| `M0121` | warn | leadership or lease heartbeat is lost | inspect PostgreSQL connectivity and HA settings |
| `M0122` | warn | verified self-update binary is staged | expect systemd restart only when explicitly enabled |

### M02xx — database and migrations

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0201` | info | migrations complete | no action |
| `M0202` | error | database operation/readiness fails | inspect `DATABASE_URL`, PostgreSQL and the detailed log |

### M03xx — authentication and login protection

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0301` | info | admin signs in | no action |
| `M0302` | warn | password or TOTP is wrong | verify credentials; investigate repetition |
| `M0303` | warn | login rate limit fires | wait for `retry_after`; check abuse |
| `M0304` | info | admin signs out | no action |
| `M0305` | warn | IP allowlist blocks a login | use an allowed management address |
| `M0306` | error | login-history persistence fails while auth continues | repair database/history writes |

### M04xx — registry, node channel and push

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0401` | debug | master dials a serve node | no action |
| `M0403` | info | a node registers online | no action |
| `M0404` | debug | a desired spec push begins | correlate `source` and node ID |
| `M0405` | info | a node applies the spec | no action |
| `M0406` | warn | a push fails | inspect agent `N####`/core output and retry after fixing |
| `M0407` | warn | a node channel disconnects or is evicted | check network, certificate and agent process |
| `M0408` | warn | handshake node ID differs from database ID | correct `HONEY_NODE_ID` or the node row |
| `M0409` | warn | heartbeat marks an enabled node down | check channel direction and listener reachability |

### M05xx — reconcile

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0501` | warn | a reconcile tick fails | inspect the detailed error; retry is automatic |
| `M0502` | debug | a node remains unreachable during reconcile | treat as context for `M0409`, not a separate incident |

### M06xx — traffic collection and retention

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0601` | debug | a node/core stats stream ends | check whether the agent or core restarted |
| `M0610` | info | traffic-history retention loop starts | no action |
| `M0611` | info | old traffic-history rows are pruned | no action |
| `M0612` | warn | traffic-history retention fails | inspect database permissions, size and migrations |

### M07xx — subscriptions

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0702` | info | a subscription token is unknown | issue a new link or check the URL |
| `M0703` | info | a subscription is disabled, expired or over quota | renew/enable/reset the user |
| `M0704` | warn | an endpoint cannot be rendered into a client link | fix endpoint domain/port/protocol fields |

### M08xx — PKI and enrollment

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0801` | info | an enrollment token is issued | deliver it once and revoke if leaked |
| `M0802` | info | a claim succeeds and a cert is issued | start the agent |
| `M0803` | warn | a claim is rejected | issue a fresh token and verify node ownership |
| `M0809` | warn | a certificate is revoked and the live channel evicted | enroll a replacement certificate |
| `M0810` | warn | presented certificate is revoked, expired or unknown | inspect certificate inventory and clock |
| `M0811` | warn | legacy CA-valid node has no enrollment inventory | migrate it through enrollment |

### M09xx — TLS and ACME

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M0901` | info | a TLS certificate reloads | no action |
| `M0902` | warn | a TLS reload fails | verify cert/key files and permissions |
| `M0903` | info | an ACME lifecycle event occurs | no action unless issuance stalls |
| `M0904` | error | ACME reports an error | inspect DNS, port 443 and ACME cache |

### M10xx — panel host/path gate

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1002` | warn | request host/path is not allowed | add the exact domain/path or fix the proxy Host |
| `M1003` | error | allowed-domain lookup fails | inspect PostgreSQL and domain records |

### M11xx — encrypted secrets

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1101` | info | secrets are re-encrypted/rekeyed | verify counts and keep old key until verified |

### M12xx — API response codes

These codes are returned as JSON `code` values. The safe public message is
deliberately less detailed than the authenticated master log.

| Code | HTTP | Meaning | Typical action |
|---|---:|---|---|
| `M1201` | 400 | invalid request or database constraint | fix payload, field bounds or uniqueness |
| `M1202` | 500 | internal API/database failure | inspect server log; do not expose detail |
| `M1203` | 502 | upstream agent request failed | inspect node channel and agent logs |
| `M1204` | 404 | resource or subscription not found | refresh IDs/token |
| `M1205` | 409 | resource conflict | use a unique tag/name or reconcile state |
| `M1206` | 410 | user/subscription gone or disabled | enable/renew/reset or issue a new token |
| `M1207` | 401 | authentication failed | sign in or replace the API key |
| `M1208` | 429 | login or request throttled | honor `Retry-After` |
| `M1209` | 403/400 | CSRF/origin policy rejected the request | use the canonical HTTPS origin |
| `M1210` | 403 | authenticated role is insufficient or address is not allowed | use the correct role/allowlist |
| `M1211` | 403 | reseller scope/entitlement rejected | operate only on owned resources |

### M13xx — managed domains

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1300` | info | domain monitor starts | no action |
| `M1301` | warn | managed certificate is near expiry | renew/repair the certificate |
| `M1302` | warn | domain listing/check fails | verify DNS, TCP 443 and certificate |

### M14xx — rolling quota windows

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1400` | info | quota scheduler starts | no action |
| `M1401` | info | a rolling quota window resets | no action |
| `M1402` | warn | quota scan/reset fails | inspect database and user state |

### M15xx — reachability and RF resilience

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1500` | info | reachability monitor starts | no action |
| `M1501` | warn | endpoint is unreachable from the master | test DNS/TCP/UDP from the master |
| `M1502` | warn | reachability scan/check fails | inspect resolver, timeout and vantage |
| `M1503` | warn | confirmed block rotates an inbound SNI | validate the new SNI from target networks |
| `M1504` | warn | pre-rollout preflight finds unreachable targets | fix reachability or review the gate policy |
| `M1505` | info | CDN rotation switches fronting host by latency | no action; verify the selected host |

### M16xx — notifications and abuse monitors

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1600` | info | Telegram bot starts | no action |
| `M1601` | warn | notification channel send fails | check target/provider credentials |
| `M1602` | warn | notification channel lookup fails | repair channel configuration |
| `M1603` | warn | Telegram polling/send fails | check bot token, network and rate limits |
| `M1604` | warn | in-app alert persistence fails | inspect database writes |
| `M1605` | warn | notification retention cleanup fails | inspect database and retention job |
| `M1610` | info/warn | traffic anomaly loop starts or scan fails; also `traffic_anomaly` alerts | investigate repeated spikes or scan errors |
| `M1611` | info/warn | device-limit monitor starts or closes excess connections | investigate sharing or Clash API health |
| `M1612` | info/warn | config-drift monitor starts or detects drift | run config preview/dry-run and push intentionally |

### M17xx — public subscription guard

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1701` | warn | hashed client/subscription bucket exceeds request budget | honor `Retry-After`; investigate abuse |
| `M1702` | warn | guard settings cannot load and safe defaults apply | repair the settings/database path |

### M18xx — scheduled operations

| Code | Level | Emitted when | Typical action |
|---|---|---|---|
| `M1800` | info | scheduler starts | no action |
| `M1801` | warn | scheduled-operation scan/mark fails | inspect the database and scheduler |
| `M1802` | warn | a scheduled operation fails to execute | inspect the operation target and detailed error |

## 7. Legacy, reserved and test-only values

These values may appear in old handoffs, old logs or tests but are not current
runtime emissions. Do not build new alerts around them.

| Code | Status | Replacement/current behavior |
|---|---|---|
| `M0105` | legacy documentation entry | tunnel currently logs an unscoped “dial acceptor up” message |
| `M0402` | legacy documentation entry | connection failures surface through the caller and `M0409`/reconcile context |
| `M0602` | legacy documentation entry | traffic retention/recording uses the current `M061x` family |
| `M0701` | legacy documentation entry | subscription serving is not currently logged with a dedicated code |
| `M1001` | legacy documentation entry | allowed panel requests are not currently logged with a dedicated code |
| `M1102` | legacy documentation entry | decrypt failures are returned through the API/internal error path |
| `M1999` | unit-test-only span search marker | never emitted by production code |
| `N0406` | invalid-filter test input | not a node diagnostic code |

`A9208` can appear inside a certificate fingerprint test value; it is not an
agent diagnostic code.

## 8. Searching and alerting

Examples:

```bash
# systemd logs
journalctl -u honey-master -u honey-agent --since '15 min ago' \
  | grep -E 'M0406|M0409|N010[236]|N011[23]'

# structured JSON deployments
journalctl -u honey-master -o json \
  | jq -r 'select(.code == "M0406") | [.MESSAGE,.node_id,.request_id] | @tsv'

# authenticated panel log search
curl -fsS -H "Authorization: Bearer $HONEY_API_TOKEN" \
  "$HONEY_BASE_URL/system/logs?code=M0406&limit=100"
```

Alert on repeated warnings/errors and on stateful incidents (`M0409`, `M0810`,
`M1301`, `M1501`, `M1701`), not on every `debug` or normal lifecycle event.
Use request IDs and node IDs to join one panel action with its agent/core
consequences.
