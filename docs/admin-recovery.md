# Admin recovery codes

Honey supports one-time recovery codes for administrators who enabled TOTP.
They are intended for the case where an authenticator app is unavailable; they
are not a replacement for the password or a second administrator.

## Operator flow

1. Sign in with the current TOTP code and open **Settings → Manage 2FA**.
2. Enter a fresh TOTP code and choose **Generate new**.
3. Store the ten displayed codes in an offline password manager or other
   protected operator storage. The panel never shows the same set again.
4. Generating a new set invalidates every older set.

Each code is 20 hexadecimal characters. Input is case-insensitive and accepts
optional spaces or a single visual dash separator. A successful recovery-code
login marks that code used atomically; a replay is rejected.

## API

- `GET /auth/totp/recovery` — returns `{ enabled, remaining }` for the current
  administrator. It never returns a code.
- `POST /auth/totp/recovery/generate` with `{ "code": "123456" }` — requires
  TOTP to be enabled and a current TOTP code. The response contains the new
  `codes` exactly once.
- `POST /auth/login` accepts `recovery_code` alongside the existing
  `totp_code`. When TOTP is enabled and no second factor is supplied, the
  response advertises `totp_required` and `recovery_available`.

Only SHA-256 digests are stored in `admin_recovery_codes`; plaintext codes are
not written to audit events, logs, backups created by the application, or API
responses after generation. Login history records invalid recovery attempts as
`bad_recovery_code`.

Recovery-code generation is deliberately protected by TOTP re-authentication.
If all codes are lost, an owner should disable and re-enable TOTP for the
account through the existing operator workflow rather than editing the table.
