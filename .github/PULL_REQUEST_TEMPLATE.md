## What changed

Describe the user-visible or operational change and why it is needed.

## Verification

- [ ] Go tests pass.
- [ ] Rust formatting, tests and relevant feature checks pass.
- [ ] UI and shell syntax checks pass.
- [ ] Database changes include a new forward-only migration.
- [ ] Protocol/config changes include generator and subscription coverage.
- [ ] Documentation and error codes are updated where behavior changed.
- [ ] No credentials, certificates, private keys, runtime data or smoke
      artifacts are included.

## Operational impact

Describe migrations, ports, configuration changes, rollout steps and rollback
steps. Write `none` when the change has no operational impact.

## Security

Do not use a pull request to disclose a vulnerability. Follow `SECURITY.md`.
