# transports

honey supports **two** connection directions between master and agent. mTLS is
identical in both (same CA, same certs); only *who dials whom* changes. On the
agent they're one interface with two impls (`agent/internal/transport`), and a
single shared `grpc.Server` — so a node can run both at once (`--mode both`).

## 1. serve — master dials the agent (default)

```
master ──TCP dial──▶ agent:8443   (agent = grpc server, TLS server)
```

- agent: `--mode serve --listen 0.0.0.0:8443`
- master: `AgentClient::connect("https://<node>:8443")` (already implemented).
- best when the node has a public, reachable port.

## 2. dial — agent dials the master (NAT-friendly)

```
agent ──TCP dial──▶ master:9443
        (agent is STILL the grpc/TLS server over that socket;
         master drives it as grpc/TLS client)
```

- agent: `--mode dial --master-addr <master>:9443` (reconnects w/ backoff).
- the trick: the agent opens the TCP socket, but the roles above the socket are
  unchanged — the agent keeps serving `AgentService`, the master keeps calling
  it. HTTP/2 already multiplexes, so no yamux/extra framing is needed. On the
  agent this is `singleConnListener` handing the dialed conn to `grpc.Server`.
- best for nodes behind NAT / with no inbound port.

### master-side counterpart (implemented, feature-gated)

The acceptor lives in [master/src/tunnel.rs](../master/src/tunnel.rs) and runs via:

```bash
(cd master && cargo run --features dial-acceptor -- dial --listen 0.0.0.0:9443)
```

what it does per accepted connection:

1. `TcpListener` on `:9443` accepts an agent connection.
2. wraps it as a **TLS client** (rustls `ClientConfig` with our CA + master
   identity, server name `honey-agent`) — mirror of `master/src/tls.rs`.
3. builds a tonic `Channel` over that already-open IO
   (`Endpoint::connect_with_connector` + `hyper_util::rt::TokioIo`).
4. runs `WhoRU`, then registers the `AgentClient` in the registry keyed by the
   node's db uuid (the agent's `--node-id`).

It's behind the `dial-acceptor` cargo feature so its extra, version-sensitive
deps (tokio-rustls, hyper-util, tower) never touch the default build. This is
the module most likely to need a small tweak on its first real compile.

## picking a mode

| node reachability            | mode   |
|------------------------------|--------|
| public ip + open port        | serve  |
| behind NAT / no inbound port | dial   |
| mixed fleet / want failover  | both   |
