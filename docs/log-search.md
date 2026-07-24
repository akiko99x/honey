# Runtime log search

The master keeps the latest 500 honey-owned tracing events in memory. The
buffer excludes dependency chatter and is intentionally non-persistent:
journald remains the authoritative long-term log.

Authenticated operators can search the ring through **Logs → Master runtime**
or `GET /system/logs`:

| Parameter | Meaning | Bound |
|---|---|---:|
| `limit` | maximum returned records, newest first | up to the 500-record ring |
| `level` | exact `error/warn/info/debug/trace` level | allowlisted |
| `code` | exact master diagnostic code in `M####` form, such as `M0406` | 5 characters |
| `q` | case-insensitive message, target, field or request ID search | 128 characters |

Filters run before the response limit, so a matching older record is not
hidden by newer unrelated events. Request-span fields are captured with each
event, which makes the `x-request-id` returned by the API searchable.

The panel debounces text input while level changes apply immediately. Issues
that expose a diagnostic code can open Logs with that code already selected;
Ctrl-K also includes **Search runtime logs**.

Runtime records are only available to authenticated non-reseller operators.
The API does not expand the existing log scope or return stored credentials.
Messages should still follow Honey's safe-error convention: secrets belong in
neither public responses nor tracing fields.

## Disposable smoke

The smoke signs in, deliberately generates one failed-login event with a known
request ID, verifies combined filters and confirms that the submitted password
does not appear:

```bash
HONEY_ADMIN_USERNAME=smoke HONEY_ADMIN_PASSWORD='long-test-password' \
bash scripts/e2e-log-search.sh
```

It creates login history and session rows, so never run it against production.
