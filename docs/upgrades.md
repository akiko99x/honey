# Upgrades & self-update

## Docker image upgrades

Docker deployments upgrade through the root-owned installer:

```bash
sudo /opt/honey-docker/scripts/install-docker.sh --upgrade
```

It creates a PostgreSQL backup, updates the pinned image tag, pulls the
master/agent/backup images and recreates the stack. The one-shot migration
container must complete before master starts. In-container binary replacement
is intentionally not used for Docker deployments.

## Legacy systemd self-update from GitHub

Panel → Settings → Software update checks GitHub Releases for a newer master
and, when enabled, installs it.

* Disabled by default. Enable **Runtime settings → Software self-update** as
  the owner. Disabled means check-only.
* Owner-only and audited. A successful install writes a `self-update` audit
  event.
* SHA-256 verification is mandatory. The selected platform asset must have an
  exact entry in the `SHA256SUMS` release asset.
* The verified binary is atomically replaced and the production systemd unit
  exits after returning the HTTP response. `Restart=always` starts the new
  binary automatically.

### Release asset layout

For a tag such as `v0.2.0`, publish:

```
honey-master-linux-x86_64
honey-0.2.0-linux-amd64.tar.gz
SHA256SUMS
```

`SHA256SUMS` must contain lines in the standard format:

```
<sha256>  honey-master-linux-x86_64
<sha256>  honey-0.2.0-linux-amd64.tar.gz
```

The updater selects the raw master binary matching the host OS and
architecture. Override the repository with `HONEY_UPDATE_REPO=owner/repo`.

### Installer

For a new single-server installation:

```bash
curl -fsSL https://raw.githubusercontent.com/akiko99x/honey/main/scripts/bootstrap.sh \
  -o /tmp/honey-bootstrap.sh
sudo bash /tmp/honey-bootstrap.sh
rm -f /tmp/honey-bootstrap.sh
```

The bootstrap installs PostgreSQL and Caddy, asks for the panel domain/path and
owner credentials, creates the database and master mTLS identity, and starts
the panel behind Caddy. It can also install verified sing-box/Xray releases and
enroll the host as the first local node. Use it on a clean Ubuntu/Debian host;
use `--force` only after reviewing the timestamped configuration backups it
creates.

The release archive can be installed directly:

```bash
sudo ./scripts/install.sh honey-0.2.0-linux-amd64.tar.gz
```

The installer preserves `/etc/honey`, installs binaries, helper scripts and
systemd units, and does not start services by default. After configuring the
environment files:

```bash
sudo ./scripts/install.sh --enable --start honey-0.2.0-linux-amd64.tar.gz
```

The installer also supports three explicit binaries:

```bash
sudo ./scripts/install.sh honey-master honey-agent honey-enroll
```

### API

* `GET /update` — owner-only check.
* `POST /update/apply` — owner-only download, checksum verification, atomic
  install and supervised restart.

### Caveats

The shipped systemd unit grants the `honey` service account write access only
to `/opt/honey/bin` and `/var/lib/honey`, which is required for the verified
in-place update. Do not enable self-update on an installation that uses a
different supervisor unless it has an equivalent `Restart=always` policy.
