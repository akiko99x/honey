# honey single-server deployment runbook

This is the repeatable path for a Linux alpha deployment. It keeps the
control plane, agent state and core configs outside `/tmp`; smoke tests may
still use `/tmp`, but that is not a production layout.

## 1. Prepare the host

Use a supported Ubuntu/Debian host with PostgreSQL, `curl`, `openssl` and
`systemd`. Before installing, verify reachability from each target network:

- TCP `22` for administration;
- TCP `443` (or the chosen VLESS/TLS port);
- the chosen UDP port for Hysteria2;
- TCP `9443` only when NAT/dial nodes are enabled.

Do not interpret a failed probe from one ISP as a honey protocol failure: first
confirm that the node IP is reachable from the intended client network.

## 2. Install immutable binaries and units

Build the three Linux binaries on a build host and copy them to the server:

```bash
sudo ./scripts/install.sh dist/honey-master dist/honey-agent dist/honey-enroll
sudo install -m 0755 /usr/local/bin/sing-box /usr/local/bin/sing-box
sudo install -m 0755 /usr/local/bin/xray /usr/local/bin/xray
```

For a published release, download and verify the archive automatically:

```bash
sudo bash scripts/install-release.sh --repo akiko99x/honey
```

The installer preserves existing `/etc/honey` configuration. Add `--start`
only after the environment files, PostgreSQL and core binary paths are ready.

The final two commands are illustrative: install the exact, separately
validated sing-box and Xray builds at the paths configured in
`/etc/honey/agent.env`. Record their versions with `sing-box version` and
`xray version` before rollout.

## 3. Configure PostgreSQL and the master key

Create `/etc/honey/master.env` with mode `0600`. Generate the key once and
store a second copy in an encrypted password store; it is independent of the
database backup:

```bash
sudo -u honey /opt/honey/bin/honey-master keygen
sudoedit /etc/honey/master.env
sudo chmod 0600 /etc/honey/master.env

set -a
source /etc/honey/master.env
set +a
/opt/honey/bin/honey-master migrate
/opt/honey/bin/honey-master admin add owner --role owner
```

Add the exact panel host and path, for example
`panel.example.com/honey`. Keep the API on loopback when a reverse proxy
terminates TLS.

## 4. Enroll the first node

Create a node in the panel, issue a one-time enrollment token, and run the
generated enrollment command as `honey`. The private key stays on the node:

```bash
sudo -u honey /opt/honey/bin/honey-enroll \
  --master https://panel.example.com/honey \
  --token '<one-time-token>' \
  --certs-dir /etc/honey/certs \
  --env-file /etc/honey/agent.env
```

Enrolled certificates are fingerprint-pinned after the first enrollment.
Revoking one in **Node → Certificates** immediately evicts the cached channel;
the same certificate cannot reconnect in either `serve` or `dial` mode. For a
no-downtime rotation, issue and install a replacement with
`honey-enroll --force`, restart the agent so it loads the replacement, verify the node is
online, and only then revoke the old certificate.

Certificates issued before the shared `honey-agent` dial SAN was introduced
should be re-enrolled before switching an existing node to `dial` mode. This
does not affect serve-mode identities whose node-specific SAN already matches.

Set `HONEY_MODE=serve` and a loopback `HONEY_LISTEN` for a co-hosted node. For
a remote node, expose the agent port only to the master. Ensure the node UUID
in `HONEY_NODE_ID` exactly matches the database node id.

## 5. Start through systemd

Configure `/etc/honey/agent.env`, including the paths to the pinned cores and
the node UUID, then start the services:

```bash
sudo systemctl enable --now honey-master.service
sudo systemctl enable --now honey-agent.service
sudo systemctl status honey-master honey-agent --no-pager
curl -fsS http://127.0.0.1:8080/ready
```

The panel should show the node online. Add an inbound and user, push, and import
the generated subscription into a known-compatible client. An ungrouped node is
available to every user; group membership restricts access at node level, never
per inbound. A successful `push` only proves control-plane reachability; also
test the public VPN endpoint from the target client network.

## 6. Restart/recovery acceptance

With an active inbound, verify that the core is running and save the current
traffic counter. Then:

```bash
sudo systemctl restart honey-agent
sudo systemctl restart honey-master
```

The agent must resume an explicitly active, unchanged config using its
`.honey-state.json` marker even if the master is temporarily unavailable.
Stopping a core deliberately writes an inactive marker; after a reboot it
must remain stopped until the next explicit push. Confirm node reconnect,
subscription availability and that traffic counters do not jump backwards or
double-count.

## 7. Backup and rollback

The installer places the backup script and systemd timer, but does not enable
services automatically:

```bash
sudo systemctl enable --now honey-backup.timer
sudo systemctl start honey-backup.service
sudo journalctl -u honey-backup.service --no-pager
```

Backups are checksummed, retain the newest 14 by default, and can be GPG
encrypted by setting `HONEY_BACKUP_GPG_RECIPIENT` after importing its public
key for the `honey` account. Test a restore before relying on them:

```bash
sudo -u honey env ADMIN_DATABASE_URL='postgres://postgres@127.0.0.1/postgres' \
  /opt/honey/bin/restore-check.sh /var/lib/honey/backups/honey-<stamp>.dump
```

Keep the matching `HONEY_SECRET_KEY` separately; it is intentionally absent
from the PostgreSQL dump. Before an upgrade, create and restore-check a backup,
keep the previous three binaries, then install the new archive and run
`honey-master migrate`. SQL migrations are forward-only: binary rollback does
not reverse schema changes. Restore the pre-upgrade database only during a
coordinated rollback with the matching old binaries and master key. Never
delete active core configs or recovery markers during a binary-only rollback.

## Troubleshooting

- `node offline`: inspect `journalctl -u honey-agent -u honey-master` and mTLS
  certificate paths before changing VPN settings.
- `push` validation error: run the core's own `check` command against the
  candidate config; the running core is preserved on validation failure.
- `sign-in required` or `too many login attempts`: wait for/reset the admin
  login throttle, then use a private browser session and the exact admin name.
- public timeout while the node is online: capture reachability from the
  client network (`Test-NetConnection`/`tcpdump`) before debugging config
  generation. An IP or route filter is outside honey's data plane.
- REALITY TCP connects but stalls before the handshake: compare a TLS request
  with the configured SNI against one without SNI and inspect the Xray debug
  log for the same client IP. Filtering can depend on the visible SNI and route;
  switching ports or transports does not prove that the server config is bad.
