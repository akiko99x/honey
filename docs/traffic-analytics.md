# Traffic history & analytics

Honey records hourly traffic buckets from the same monotonic `node + user +
core` deltas used for quota enforcement. The write happens in the transaction
that advances `node_user_traffic`, so an agent restart or a new stats epoch
cannot duplicate usage.

## API

`GET /analytics/traffic` accepts:

- `from` and `to` — RFC3339 timestamps; defaults to the previous 24 hours;
- `bucket=hour|day` — longer ranges default to day buckets;
- `node_id`, `user_id`, `core=singbox|xray` — optional filters.

The range is limited to 366 days and hourly queries to 31 days. The response
contains upload/download totals, a previous-period comparison, time-series
points, top users, core breakdown and (for infrastructure roles) current fleet
health. Resellers receive only their own users and do not receive node or fleet
health data.

`GET /analytics/traffic.csv` exports the selected time-series as a bounded CSV.
Both routes accept a panel session or a scoped bearer API key with viewer scope.

## Retention

Owners can change **Traffic history, days** in Settings. Values are clamped to
7–3650 days; the default is 180. A background task removes expired buckets every
six hours. Quota resets clear live counters but intentionally keep history.

The panel deliberately calls the core split `sing-box`/`xray`, not a protocol
breakdown: the current agent stats contract reports per-user, per-core counters,
and cannot attribute a shared user to a particular inbound without a proto
change. Geo, IP tracking, billing and live connections remain separate future
items.

## Disposable smoke

On a disposable Linux host with the usual `DATABASE_URL`, `HONEY_API_TOKEN`,
`MASTER_URL` and `ADMIN_PASSWORD` environment variables, run:

```sh
./scripts/e2e-traffic-analytics.sh
```

The scenario checks migration-backed analytics, bounded ranges and CSV content.
It does not touch production infrastructure.
