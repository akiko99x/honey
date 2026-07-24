# honey — operations runbooks

Short, do-this procedures for the boring-but-critical bits. See
`docs/error-codes.md` for the log codes referenced here.

## Master key rotation (`HONEY_SECRET_KEY`)

honey encrypts user UUIDs/passwords, REALITY private keys and admin TOTP secrets
at rest under `HONEY_SECRET_KEY`. To rotate the key:

1. **Back up first** — `scripts/backup-postgres.sh` and copy the *old* key
   somewhere safe. If rekey is interrupted, you restore + retry with the old key.
2. Generate the new key: `NEW=$(honey-master keygen)`.
3. Stop the master (`systemctl stop honey-master`) so nothing writes new
   ciphertext mid-rotation.
4. Re-encrypt every secret from old to new:
   ```bash
   HONEY_SECRET_KEY_OLD=<old-key> HONEY_SECRET_KEY=$NEW \
     honey-master rekey --database-url "$DATABASE_URL"
   ```
   It rewrites user, inbound and admin secrets and prints the counts.
5. Put `$NEW` into `/etc/honey/master.env` as `HONEY_SECRET_KEY`, then
   `systemctl start honey-master`.
6. Destroy the old key once you have confirmed logins and subscriptions work.

`honey-master reencrypt` is the *same-key* variant — use it to encrypt a legacy
plaintext database after first setting `HONEY_SECRET_KEY`.

**Losing the key makes those secrets unrecoverable.** A database backup without
the matching key is useless.

## Scheduled backups + verified restore

- **Backups**: `scripts/backup-postgres.sh` writes a checksummed custom-format
  dump, prunes to `HONEY_BACKUP_KEEP` (default 14), and gpg-encrypts if
  `HONEY_BACKUP_GPG_RECIPIENT` is set. Install the timer:
  ```bash
  install -m755 scripts/backup-postgres.sh /opt/honey/bin/
  install -m644 deploy/systemd/honey-backup.{service,timer} /etc/systemd/system/
  systemctl daemon-reload && systemctl enable --now honey-backup.timer
  ```
  It runs daily ~03:30 UTC; check with `systemctl list-timers honey-backup`.
  `scripts/install.sh` installs both scripts and units; it intentionally leaves
  the timer disabled until the operator has checked `DATABASE_URL`, retention,
  and the optional GPG recipient in `/etc/honey/master.env`.
- **Verify the restore** (a backup you never restore is a rumour). On a schedule
  or before an upgrade:
  ```bash
  ADMIN_DATABASE_URL=postgres://postgres@127.0.0.1/postgres \
    scripts/restore-check.sh /var/lib/honey/backups/honey-<stamp>.dump
  ```
  It loads the dump into a throwaway database, prints the row counts and drops it.
- Store the encrypted backups **and** `HONEY_SECRET_KEY` off-box, separately.
- CI also runs `scripts/secret-recovery-check.sh` against a disposable restored
  database. It proves that the saved key decrypts credentials, a wrong key
  fails closed, and the restore survives `rekey`. The same rehearsal can be run
  manually with `DATABASE_URL`, `ADMIN_DATABASE_URL`, `HONEY_SECRET_KEY`, and a
  built `honey-master` binary.

## Release archive and migration rehearsal

`scripts/package-release.sh` packages already-built Linux amd64 binaries with
the deployment files and writes a sibling `.sha256`. GitHub tags matching
`v*.*.*` run the same packaging in `.github/workflows/release.yml`.

CI exercises three database paths: all migrations on an empty database, an
upgrade from the first 12 registered SQLx migrations, and a real pg_dump restore
into a throwaway database. Before a production upgrade, repeat the restore with
the actual encrypted artifact and keep `HONEY_SECRET_KEY` out of that backup.

## Log rotation / journald retention

honey logs to stdout/stderr, captured by journald — no log files to rotate.
Cap journald so logs can't fill the disk (`/etc/systemd/journald.conf`):

```ini
[Journal]
SystemMaxUse=500M
MaxRetentionSec=30day
```

Then `systemctl restart systemd-journald`. Per-service logs:
`journalctl -u honey-master -f`. For machine-readable logs set
`HONEY_LOG_FORMAT=json`; filter by level with `RUST_LOG` (e.g. `RUST_LOG=warn`).
The master also keeps a live in-memory tail at `GET /system/logs` (panel →
Logs). It supports bounded `level`, exact `code`, free-text/request-ID `q`
and `limit` filters; see [log-search.md](log-search.md).

For external automation, create a named bearer key in Settings instead of
sharing `HONEY_API_TOKEN`. Keys are hashed at rest, role-scoped, optionally
expiring and immediately revocable; `/openapi.json` describes the supported
surface. See [api-keys.md](api-keys.md).

## Issues and alerts

The panel **Issues** page is the current-state cockpit. It aggregates node,
push, inbound, managed-domain, agent-certificate and suppressed-user health.
Use its severity/type/node filters, open the affected entity, or run one of the
safe actions (push preview/retry, endpoint probe, domain verification). A fixed
condition disappears on refresh; historical events remain in Logs/audit.

The same snapshot is available to authenticated viewer-or-higher operators at
`GET /issues`. Prometheus exposition includes the gauges
`honey_issues{severity="critical|warning|info"}`. Resellers do not receive the
fleet-wide endpoint because it would cross their ownership boundary.

These conditions also surface as coded log lines you can watch via
`journalctl -p warning -u honey-master` or the panel Logs tab:

- `M0409` node went down (heartbeat failed);
- `M0406` push to a node failed;
- `M1301` a managed domain's certificate expires within 14 days;
- `M1401`/`M1402` quota window reset / reset failure.

The panel bell retains the four primary events (`M0409`, `M0406`, `M1301`,
`M1401`) for 90 days with per-admin read state. Use it for recent history and
navigation to the affected entity; use **Issues** for the current condition.
Resellers cannot see the fleet-wide notification stream.

Issues are derived from current database state and are not a paging history.
Do not alert on `info` by default: intentional disabled users are reported at
that severity for inventory visibility. A revoked historical node certificate
also does not stay open when the node has another currently valid certificate.

## Admin hardening

- **Two-factor**: panel → Settings → Two-factor → scan the QR, confirm a code.
  Logins then require the 6-digit code.
- **IP allowlist**: panel → Settings → IP allowlist. Empty means open; add your
  own address/CIDR **before** relying on it or you can lock yourself out. It is
  fail-open on a database error to avoid a self-lockout.
- **Compromised admin session**: panel → Settings → Sessions & login history.
  Review the source IP, user agent and recent failed logins, then revoke the
  suspicious session or use *Revoke all other sessions*. Revoke the current
  session last because it immediately signs the operator out. An admin/owner
  can inspect and revoke sessions for another admin from *Manage admins*;
  resellers remain limited to their own account.
- **Periodic quotas**: a user page → Quota window → daily/weekly resets the
  traffic counter on that cadence (scheduler runs every 5 min).

## Reachability & Hysteria2 port hopping

- The master TCP-probes each inbound's public port every ~2 min and on demand
  (inbound page → *Probe reachability*); confirmed-down endpoints drop out of
  subscriptions. UDP protocols (hy2/tuic) can't be probed from the master — run
  an external checker in the target region that `PUT`s `/inbounds/:id/reachability`
  (`{"reachable":true|false}`) with the API bearer token.
- **Hysteria2 port hopping**: set `hop_ports` (e.g. `20000-30000`) on the inbound.
  The client link carries it (`mport` / `server_ports`); the agent installs an
  isolated nftables redirect (`table inet honey`) from that UDP range to the
  listen port. Requires `nft` and `CAP_NET_ADMIN` (the systemd unit grants it).
  Verify: `nft list table inet honey`. If `nft` is missing it's logged (`N0304`)
  and you can add the REDIRECT manually.
