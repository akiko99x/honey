# Public subscription guard

Every public subscription document is protected by a live, in-memory request
budget. The guard covers the universal URL, client-specific formats, QR
endpoints and custom aliases:

- `/sub/:token` and every `/sub/:token/*` format;
- `/s/:alias`.

The bucket key combines a one-way client-address hash with the resolved user
identity. A token and its alias therefore share one budget, while two clients
do not consume each other's budget. Invalid tokens and aliases are also
bounded. Honey never writes a raw client address, token or alias to the guard
log or notification.

When the budget is exhausted, the master returns `429 Too Many Requests` with
`Retry-After`, a safe JSON error carrying `M1701`, and private/no-store
headers. Successful subscription responses receive the same cache,
referrer-policy and content-sniffing protection.

## Runtime settings

Owners can change the following values in **Settings → Runtime settings**
without restarting the master:

| Setting | Default | Accepted range |
|---|---:|---:|
| enabled | `true` | boolean |
| requests per window | 120 | 10–10,000 |
| window | 60 seconds | 10–3,600 seconds |
| block | 300 seconds | 10–86,400 seconds |

The dialog exposes allowed/blocked counters since process start, active
buckets and recent persisted block occurrences. Disabling the guard is meant
only for short diagnostics.

Repeated blocks create a deduplicated `subscription_abuse` notification and
an Issues warning for the recent 30-minute window. Persistence keeps abuse
visible after a restart without retaining the sensitive bucket key.

## Disposable smoke

Use a fresh active subscription belonging to a disposable database. The smoke
temporarily changes runtime settings, intentionally blocks the subscription
and restores the previous settings on exit:

```bash
HONEY_ADMIN_USERNAME=smoke HONEY_ADMIN_PASSWORD='long-test-password' \
HONEY_SUB_URL='http://127.0.0.1:8080/sub/<disposable-token>' \
bash scripts/e2e-subscription-guard.sh
```

Never run this scenario against a production subscription.
