# Admin sessions and login history

Honey stores browser sessions as SHA-256 token hashes. The original cookie
token is never persisted and neither the token nor its hash is returned by the
session-management API.

Open **Settings → Sessions & login history** to review the current account's
active sessions. Each row shows the bounded User-Agent, remote address,
creation/last-seen time, and expiry. A session can be revoked individually;
revoking the current session also expires the browser cookie. **Revoke all
others** preserves the current browser and terminates the account's other
sessions.

Owners can open an administrator from **Manage admins → sessions**. Admin-role
API clients may also inspect or revoke another account's sessions. Operators,
viewers, and resellers remain limited to their own account.

## API

- `GET /auth/sessions[?admin_id=<uuid>]`
- `DELETE /auth/sessions/{id}`
- `POST /auth/sessions/revoke-others`
- `GET /auth/login-history[?admin_id=<uuid>&limit=100]`
- `GET /auth/login-history?all=true&limit=200` (admin/owner only)

These endpoints require a panel session. The legacy bearer token has no account
identity and cannot list or own browser sessions.

Login history records `success`, `bad_credentials`, `bad_totp`, `ip_denied`,
and `rate_limited` outcomes. User-Agent and address fields are length-bounded;
passwords, TOTP values, cookies, and token hashes are never recorded. Events
older than 90 days and expired sessions are pruned during subsequent login
activity. History begins after migration `0025`; it is not reconstructed from
older logs.

For disposable-environment acceptance, run:

```bash
HONEY_BASE_URL=http://127.0.0.1:8080 \
HONEY_ADMIN_USERNAME=owner \
HONEY_ADMIN_PASSWORD='test-password' \
scripts/e2e-admin-sessions.sh
```

Set `HONEY_ADMIN_TOTP` when the test account requires TOTP. The smoke revokes
all other sessions for that account, so never point it at a production owner.
