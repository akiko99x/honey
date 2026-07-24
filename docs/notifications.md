# In-app notifications

Honey persists the same operational alerts that can be sent to webhook,
Discord, Slack or Telegram channels. The bell in the panel is therefore useful
even when no external channel is configured.

## Events and navigation

| Event | Code | Severity | Panel destination |
|---|---|---|---|
| node heartbeat failure | `M0409` | critical | node detail |
| configuration push failure | `M0406` | critical | node detail |
| certificate near expiry | `M1301` | warning | Domains |
| periodic quota reset | `M1401` | info | user detail |
| public subscription rate limit | `M1701` | warning | Issues |

Events with the same dedupe key are collapsed for 30 minutes. A repeated
condition increments `occurrence_count` and refreshes `last_seen_at`; after the
cooldown a new unread event is created. Events older than 90 days are deleted
by a six-hour retention task (and opportunistically when alerts are recorded).
Read state is stored separately for each admin.

The list API supports `severity`, `event`, `unread` and a bounded `limit`:

- `GET /notifications`
- `GET /notifications/unread-count`
- `POST /notifications/:id/read`
- `POST /notifications/read-all`

Viewer, operator, admin and owner panel sessions can read and acknowledge
events. The legacy bearer can inspect but cannot create personal read state.
Resellers cannot access this system-wide stream because node/domain details are
outside their tenant scope. Dedupe keys are never serialized by the API.

## Disposable smoke

The smoke inserts synthetic rows into the master's database and marks every
notification read for the test account. Never point it at production:

```bash
DATABASE_URL=postgres://honey:password@127.0.0.1/honey_test \
HONEY_ADMIN_USERNAME=smoke HONEY_ADMIN_PASSWORD='long-test-password' \
bash scripts/e2e-notifications.sh
```
