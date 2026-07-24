# Scoped API keys

Honey supports named bearer keys for CI and external integrations. Keys use the
same linear permission ladder as panel accounts: `viewer`, `operator`, `admin`,
and `owner`. Reseller scope is intentionally unavailable to API keys because
reseller access also depends on per-object ownership and group entitlements.

Only an owner may list, create, or revoke keys. A key can never be created above
the creator's role. The plaintext token starts with `hny_`, is returned once,
and is never stored by the master; PostgreSQL contains only its SHA-256 digest.

## Create and use a key

Open **Settings → API keys**, choose a name, role and expiry in days. `0` means
no expiry. Copy the token before closing the one-time result dialog.

```sh
curl -H 'Authorization: Bearer hny_REPLACE_ME' \
  https://panel.example.com/honey/nodes
```

The API description is public at `/openapi.json`. It documents the stable
automation surface, request schemas, role requirements and common error body:

```json
{"error":"operator role required","code":"M1210"}
```

Every response also carries `x-request-id`; use it to correlate API failures
with **Logs → Master runtime**.

## Lifecycle and operational safety

- `active`: authentication is accepted;
- `expired`: expiry has passed and authentication returns `401`;
- `revoked`: explicitly disabled and authentication returns `401`.

`last_used_at` is updated at most once per five minutes. Authentication and the
throttled touch happen in one database round trip, so high-frequency read-only
automation does not write the key row on every request.

Create and revoke operations are written to the audit log. List responses never
contain the token or digest. Revocation is immediate, so rotate an integration
to a replacement key before revoking its current key.

The legacy `HONEY_API_TOKEN` remains owner-scoped for compatibility. New
automation should use named keys so access can be expired and revoked without a
master restart.

## Disposable smoke

Never run the lifecycle smoke against production: it creates real keys and
intentionally exercises rejected requests.

```sh
HONEY_BASE_URL=http://127.0.0.1:8080 \
HONEY_ADMIN_USERNAME=test-owner \
HONEY_ADMIN_PASSWORD='long-test-password' \
scripts/e2e-api-keys.sh
```

The smoke proves one-time token handling, viewer/admin RBAC, `last_used_at`,
input bounds, revoke invalidation, audit events and OpenAPI availability.
