# Multihop / cascade

Chain a user's traffic through a second node before it egresses: users connect
to an **entry** inbound (e.g. a whitelisted VPS in one country), and their
traffic exits from an **exit** inbound on another node (e.g. abroad). Both must
run on **sing-box**.

## How to set it up

1. Create the **exit** inbound normally on the exit node (any chainable
   protocol: vless, vmess, trojan, hysteria2, tuic, shadowsocks).
2. On the **entry** inbound, set **Multihop exit** to that inbound.

That's it — users keep using the entry inbound's subscription; their traffic now
egresses from the exit node.

## What happens under the hood

* Setting the exit generates a dedicated **chain credential** on the entry
  inbound (a UUID + an encrypted password).
* The master builds a sing-box **outbound** to the exit (reusing the same
  outbound builder as client subscriptions) tagged `chain-<entry_tag>`, and adds
  a **route rule** so the entry inbound's traffic goes through it instead of
  `direct`.
* The exit inbound gains the chain credential as one of its users, so the hop
  authenticates like any other client.

The entry node reaches the exit over the exit's public path — including a CDN
host header when the exit uses a ws/http transport with one.

## Constraints

* Entry and exit are **sing-box** (the chain outbound is sing-box). An xray
  entry or a non-chainable exit protocol is rejected.
* An inbound cannot chain to itself, and a direct two-node cycle (A→B while
  B→A) is rejected.
* One hop is modelled per inbound. Deeper chains (A→B→C) are not built
  automatically; point several entries at one exit for fan-in.
* This can't be verified with a live tunnel in CI — the config emission, the
  outbound/route injection and the credential flow are unit-tested; the live
  handshake needs two real sing-box nodes.
