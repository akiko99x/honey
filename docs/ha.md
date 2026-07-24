# Multi-master HA

Run several `honey-master` instances against one PostgreSQL. Every instance
serves the API and panel; exactly one — the **leader** — runs the singleton
background loops. Election needs no extra infrastructure: it is a single-row
lease in the database (migration `0046_ha.sql`).

## How it works

* Each process gets an instance id at boot and heartbeats into `ha_instances`.
* Every `ttl/3` seconds it tries to take or renew the lease in `ha_leader` with
  one atomic upsert. Another instance can only take over **after the lease has
  actually expired** — that condition is in the statement's `WHERE`, so two
  instances can never both hold it.
* A leader that fails to renew (database unreachable, network partition) **steps
  down immediately**, before the lease expires elsewhere. It stops acting rather
  than racing a new leader.
* A single-instance deployment simply always wins, so non-HA setups behave
  exactly as before.

Failover delay is at most the lease TTL: `HONEY_HA_LEASE_SECS` (default `15`,
clamped to 5–300).

## What is leader-only

Loops that would double-push, double-count or duplicate alerts:

reconcile · stats collection · quota · scheduled operations · reachability
monitor · domain monitor · traffic & notification retention · anomaly, status
sampler, device-limit and config-drift monitors · Telegram bot long-poll.

Everything else (the REST API, panel, subscriptions, status page) runs on every
instance — that is the point of HA.

## Requirements

* **One shared PostgreSQL** for all instances (its own HA is a separate concern).
* **The same `HONEY_SECRET_KEY`** (or the same secret backend) on every
  instance — otherwise an instance cannot decrypt secrets written by another.
* **The same node certificate directory** (`--certs-dir`), shared or replicated:
  the leader dials agents with those client certificates, and any instance may
  become leader.
* A load balancer in front for the panel/API and subscription traffic.

## Known limits

* **Dial-mode (NAT) nodes**: the agent dials *one* address and only the leader
  pushes. Point the dial endpoint at the leader (or a leader-aware address);
  round-robining it across instances will leave such nodes unconverged. Serve-mode
  nodes are unaffected — the leader dials them.
* **Panel ACME**: the built-in ACME loop obtains a certificate per instance for
  the instance's own HTTPS listener. For multi-instance setups terminate TLS at
  the load balancer instead, or give each instance its own hostname.
* Failover is not instant — background work pauses for up to the lease TTL.

## Checking status

`GET /ha` (and **Settings → High availability** in the panel) shows the instance
roster, which one is leader, and when the current lease expires.
