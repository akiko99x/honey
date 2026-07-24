# Release checklist

The repository automation is prepared, but a tag must not be pushed until every
item below is complete.

For the first public push, follow
[github-publication.md](github-publication.md) before this tag checklist.

Run `bash scripts/release-readiness.sh --full` on the release commit. On Linux,
`--package` additionally builds and inspects the exact checksummed archive.
Before Git initialization, `--allow-no-git` permits source checks but does not
satisfy the repository/history gate.

On Linux with a maintenance PostgreSQL URL, rehearse enrollment, mTLS,
candidate preview/dry-run, apply, and master-independent agent recovery in
fully disposable state:

```bash
ADMIN_DATABASE_URL=postgres://honey:password@127.0.0.1:5432/postgres \
  HONEY_MASTER_BIN=master/target/debug/honey-master \
  bash scripts/lifecycle-linux.sh
```

The harness uses a scratch database, temporary PKI and fake core processes. It
does not write `/etc/honey`, install units, or mutate firewall rules.

## Repository

- Initialize the real Git repository and review the complete first commit.
- Run the secret scan on the full history and inspect every finding.
- Confirm that `LICENSE`, package metadata, the README, and the release archive
  consistently identify `AGPL-3.0-only`.
- Enable private vulnerability reporting or add a monitored contact to
  `SECURITY.md`.
- Require the `quality`, `database-recovery`, and `secrets` CI jobs on the
  protected default branch.

## Build and data

- All CI jobs pass on the exact release commit.
- A production-like encrypted backup passes `restore-check.sh` with the matching
  off-box `HONEY_SECRET_KEY` available separately.
- The migration upgrade rehearsal passes and the operator has retained the old
  binaries plus a pre-upgrade database backup.
- On a disposable account, `scripts/e2e-admin-sessions.sh` proves session
  listing, individual revoke, revoke-others, login history, and current-cookie
  invalidation. Never run it against a production owner account.
- Against the same disposable database, `scripts/e2e-notifications.sh` proves
  notification listing, per-admin read state, unread counts and safe API output.
  It inserts synthetic events and marks all events read, so never run it on production.
- With a fresh disposable subscription, `scripts/e2e-subscription-guard.sh`
  proves the public request budget, `429` + `Retry-After`, private response
  headers, safe error output, telemetry and the deduplicated `M1701`
  notification. It temporarily lowers live limits and must never target production.
- On a disposable admin, `scripts/e2e-log-search.sh` proves combined
  level/code/request-ID filtering, bounded invalid-query rejection and that a
  submitted password is absent from runtime-log responses.
- On a disposable owner, `scripts/e2e-api-keys.sh` proves one-time token
  creation, viewer/admin RBAC, throttled usage tracking, bounded expiry/scope
  validation, revoke invalidation, audit events and the public OpenAPI document.
- On a disposable admin with TOTP enabled, run
  `scripts/e2e-admin-recovery.sh`: it generates a fresh recovery-code set,
  verifies that only digests exist in `admin_recovery_codes`, signs in with one
  code, and confirms that replay is rejected and login history records
  `bad_recovery_code` for an invalid attempt. Recovery codes are shown once.
- `scripts/e2e-api.sh` reaches `5/5` derived onboarding progress in the order
  domain → node → inbound → user → subscription and removes its synthetic
  `.invalid` domain during cleanup. Run it only against a disposable master.
- `master/Cargo.toml` version equals the intended `vX.Y.Z` tag.

## Compatibility and acceptance

- Record exact sing-box, Xray, Linux distribution, and tested client versions.
- Complete lifecycle acceptance: enrollment, push, traffic counters,
  quota/reset, disable/expiry, token rotation, certificate revoke, and
  restart/offline recovery. The disposable CI harness covers
  enrollment, mTLS, revoke/re-enrollment, push and offline recovery; real
  traffic growth and enforcement still require the release Linux stand.
- Repeat the short VLESS+REALITY and Hysteria2 network regression from the target
  networks after the clean package install. Keep smoke credentials and runtime
  files outside the repository.

## Publication

- Push the signed or protected tag only after the items above are approved.
- Confirm the release workflow publishes one Linux amd64 archive and its
  `.sha256`, download both, and verify the checksum independently.
- Review generated release notes for internal hostnames, addresses, paths, or
  operational details before making the release public.
