# Publishing honey on GitHub

This runbook covers the first public push, repository hardening and the first
automated binary release. It assumes the canonical repository is
`akiko99x/honey`, the default branch is `main`, and the first public tag is
`v0.0.0`. Replace the slug everywhere before the first commit if a different
GitHub owner will host the project.

Publishing is intentionally split into two events:

1. push the reviewed source to a **private** GitHub repository;
2. enable security controls, make the repository public, then push the release
   tag.

This keeps an accidental source or metadata problem out of the first public
snapshot. It does not make committed secrets safe: rotate a leaked secret
before doing anything else.

## 1. Prerequisites

Install Git and the official GitHub CLI package on the Ubuntu/Debian machine
that holds the canonical source:

```bash
sudo apt-get update
sudo apt-get install -y git curl ca-certificates ripgrep

sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL \
  https://cli.github.com/packages/githubcli-archive-keyring.gpg \
  -o /tmp/githubcli-archive-keyring.gpg
sudo install -m 0644 \
  /tmp/githubcli-archive-keyring.gpg \
  /etc/apt/keyrings/githubcli-archive-keyring.gpg
rm -f /tmp/githubcli-archive-keyring.gpg

printf '%s\n' \
  "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
  | sudo tee /etc/apt/sources.list.d/github-cli.list >/dev/null

sudo apt-get update
sudo apt-get install -y gh
```

The maintained installation commands and signing-key fingerprints are in the
official
[`cli/cli` Linux installation guide](https://github.com/cli/cli/blob/trunk/docs/install_linux.md).
Authenticate the CLI in your own GitHub account:

```bash
gh auth login --hostname github.com --git-protocol https --web
gh auth status
git --version
gh --version
```

The commands below use the GitHub CLI's existing-source workflow:
`gh repo create OWNER/REPO --source=. --push`. See the official
[`gh repo create` manual](https://cli.github.com/manual/gh_repo_create).

Set the intended public identity:

```bash
export HONEY_GITHUB_OWNER=akiko99x
export HONEY_GITHUB_REPO=honey
export HONEY_GITHUB_SLUG="$HONEY_GITHUB_OWNER/$HONEY_GITHUB_REPO"
printf '%s\n' "$HONEY_GITHUB_SLUG"
```

The output must be `akiko99x/honey`. Confirm that the canonical slug is
consistent before committing:

```bash
rg -n 'akiko99x/honey' \
  README.md SECURITY.md CHANGELOG.md docs scripts deploy master
```

The Go module and generated protobuf metadata use
`github.com/akiko99x/honey/agent`, matching the repository subdirectory.

## 2. Source and secret preflight

Run this from the server-side source tree, not from `/opt/honey`:

```bash
cd /home/user/honey

bash scripts/release-readiness.sh --full --allow-no-git
```

On Linux, the stronger package check builds and inspects the exact release
archive:

```bash
bash scripts/release-readiness.sh --package --allow-no-git
```

If Docker is available, scan the source tree before creating history:

```bash
docker run --rm \
  -v "$PWD:/repo" \
  ghcr.io/gitleaks/gitleaks:v8.30.1 \
  dir --redact --verbose /repo
```

Manually confirm that none of these are present:

- `/etc/honey` copies, PostgreSQL dumps or runtime databases;
- certificates, CA material, private keys or enrollment bundles;
- `.env` files, API keys, admin passwords or `HONEY_SECRET_KEY`;
- subscription URLs/tokens, VPN UUIDs/passwords or REALITY private keys;
- real smoke hostnames, public IP addresses, usernames or packet captures;
- `build/`, `dist/`, `master/target/`, logs or editor/assistant state.

Only synthetic values belong in examples. If a live value has ever been copied
into this source tree, rotate it even when `.gitignore` excludes the file.

## 3. Initialize and review the first commit

The current source tree can contain an empty `.git` directory left by earlier
preparation. Remove it only after proving that it is empty:

```bash
cd /home/user/honey

if [ -d .git ] && [ -z "$(find .git -mindepth 1 -print -quit)" ]; then
  rmdir .git
fi

git init -b main
git config user.name "akiko"
git config user.email "YOUR VERIFIED GITHUB OR NOREPLY EMAIL"
```

The commit email becomes public with the repository. Use the address shown in
GitHub **Settings → Emails**. Choose GitHub's `noreply` address instead of a
personal mailbox when email privacy matters.

Stage everything, then review the exact public snapshot:

```bash
git add -A
git status --short
git diff --cached --check
git diff --cached --stat
git diff --cached --name-only
```

The staged list must not contain `build/`, `dist/`, `master/target/`, `.env`,
certificates, keys, dumps, databases, logs, packet captures or local assistant
state. The source-tree secret scan in the previous section must already be
clean.

Commit only after the staged snapshot is accepted:

```bash
git commit -m "Initial public release"
git status --short --branch
git log --oneline --decorate -1
```

Now scan the complete reachable Git history:

```bash
docker run --rm \
  -e GIT_CONFIG_COUNT=1 \
  -e GIT_CONFIG_KEY_0=safe.directory \
  -e GIT_CONFIG_VALUE_0=/repo \
  -v "$PWD:/repo" \
  ghcr.io/gitleaks/gitleaks:v8.30.1 \
  git --redact --verbose /repo
```

The working tree and history scan must be clean before upload. If commit
signing is already configured, use `git commit -S` and later `git tag -s`; do
not invent signing configuration during release.

## 4. Create a private GitHub repository and push

The repository must be empty on GitHub: do not create a README, `.gitignore` or
license in the GitHub form. Create it from the reviewed local history:

```bash
gh repo create "$HONEY_GITHUB_SLUG" \
  --private \
  --source=. \
  --remote=origin \
  --push \
  --description "Universal multi-node VPN panel with sing-box and Xray"
```

Verify both local and remote state:

```bash
git remote -v
git branch -vv
gh repo view "$HONEY_GITHUB_SLUG" --web
gh run list --repo "$HONEY_GITHUB_SLUG" --limit 10
```

The `ci` workflow should start on the initial push. Do not make the repository
public while `quality`, `database-recovery` or `secrets` is failing.

## 5. Configure the GitHub repository

Apply the normal repository metadata:

```bash
gh repo edit "$HONEY_GITHUB_SLUG" \
  --description "Universal multi-node VPN panel with sing-box and Xray" \
  --enable-issues \
  --enable-discussions \
  --delete-branch-on-merge \
  --add-topic vpn \
  --add-topic sing-box \
  --add-topic xray \
  --add-topic rust \
  --add-topic golang \
  --add-topic self-hosted
```

In GitHub **Settings → Actions → General**:

- keep workflow permissions at **Read repository contents**;
- allow write access only where a workflow declares it; `release.yml` declares
  `contents: write`;
- do not allow unreviewed actions from arbitrary sources.

In **Settings → Rules → Rulesets**, create a branch ruleset for `main`:

- require a pull request before merging;
- require the `quality`, `database-recovery` and `secrets` checks;
- require conversations to be resolved;
- block force pushes and branch deletion;
- optionally require signed commits if every maintainer is configured for it;
- leave an owner bypass only if emergency recovery requires it.

In **Settings → Code security and analysis**:

- enable Dependabot alerts and security updates;
- enable secret scanning and push protection;
- enable private vulnerability reporting.

The repository includes Dependabot configuration, issue forms, a pull request
template, generated release-note categories and `SECURITY.md`. Private
vulnerability reporting must be enabled in GitHub itself; a repository file
cannot enable that server-side feature.

## 6. Make the reviewed repository public

After CI is green and the GitHub settings are complete:

```bash
gh repo edit "$HONEY_GITHUB_SLUG" \
  --visibility public \
  --accept-visibility-change-consequences
```

Then enable GitHub's public-repository secret controls from the CLI where the
account plan supports them:

```bash
gh repo edit "$HONEY_GITHUB_SLUG" \
  --enable-secret-scanning \
  --enable-secret-scanning-push-protection
```

If either flag is unavailable for the account, verify the equivalent setting in
the web UI. Open the repository in a signed-out/private browser window and
check that the README, license, security policy, contribution guide, issue
forms and Actions tab are public.

## 7. Publish `v0.0.0`

The tag is a separate release gate. First confirm the source version and clean
tree:

```bash
cd /home/user/honey

grep '^version = ' master/Cargo.toml | head -1
grep '^const agentVersion' agent/internal/grpcserver/server.go
git status --short --branch
bash scripts/release-readiness.sh --full
```

Both component versions must be `0.0.0`. Push an annotated tag:

```bash
git tag -a v0.0.0 -m "honey v0.0.0"
git push origin v0.0.0
```

Do not run `gh release create` for the normal path. Pushing `v0.0.0` triggers
`.github/workflows/release.yml`, which tests the release commit, builds all
three Linux amd64 binaries, creates the checksummed archive and publishes the
GitHub Release.

Watch the workflow:

```bash
gh run list \
  --repo "$HONEY_GITHUB_SLUG" \
  --workflow release.yml \
  --limit 5

gh run watch RUN_ID \
  --repo "$HONEY_GITHUB_SLUG" \
  --exit-status
```

Inspect the published release:

```bash
gh release view v0.0.0 \
  --repo "$HONEY_GITHUB_SLUG" \
  --web

rm -rf /tmp/honey-release-v0.0.0
mkdir -p /tmp/honey-release-v0.0.0

gh release download v0.0.0 \
  --repo "$HONEY_GITHUB_SLUG" \
  --dir /tmp/honey-release-v0.0.0

cd /tmp/honey-release-v0.0.0
sha256sum -c SHA256SUMS
sha256sum -c honey-0.0.0-linux-amd64.tar.gz.sha256
tar -tzf honey-0.0.0-linux-amd64.tar.gz
```

The release should contain:

- `honey-master-linux-x86_64`;
- `honey-0.0.0-linux-amd64.tar.gz`;
- `honey-0.0.0-linux-amd64.tar.gz.sha256`;
- `SHA256SUMS`.

The GitHub CLI also supports manual releases through
[`gh release create`](https://cli.github.com/manual/gh_release_create), but use
that only as a documented recovery path after understanding why the automated
workflow failed.

## 8. Verify public installation and self-update

Use a clean disposable Ubuntu/Debian host, not the smoke server with preserved
state:

```bash
curl -fsSL \
  https://raw.githubusercontent.com/akiko99x/honey/main/scripts/bootstrap.sh \
  -o /tmp/honey-bootstrap.sh

sudo bash /tmp/honey-bootstrap.sh \
  --repo akiko99x/honey \
  --version v0.0.0

rm -f /tmp/honey-bootstrap.sh
```

Confirm:

- the archive checksum is verified before installation;
- PostgreSQL, Caddy, master, agent and both cores are installed as selected;
- the panel is reachable at the chosen HTTPS domain and path;
- health/ready pass, the node is online and a push applies;
- VLESS+REALITY and Hysteria2 work from an external client;
- backup plus scratch restore succeeds;
- Settings reports repository `akiko99x/honey`;
- self-update remains check-only until explicitly enabled.

For a repository hosted under another owner, pass `--repo OWNER/honey` and set
`HONEY_UPDATE_REPO=OWNER/honey` in the installed master environment.

## 9. Normal release flow after `v0.0.0`

For every later release:

1. update `master/Cargo.toml`, the `honey-master` entry in `master/Cargo.lock`,
   `agentVersion`, `web/openapi.json` and `CHANGELOG.md`;
2. run `bash scripts/release-readiness.sh --package`;
3. merge through a green pull request;
4. create and push `vX.Y.Z` matching the Cargo version exactly;
5. watch `release.yml`, verify downloaded checksums, then test a clean install
   and an upgrade;
6. review generated notes and publish any operational caveats.

Never move or reuse a published tag. Fix the source and publish a new semantic
version.

## 10. Recovery and rollback

If the initial private push contains a secret:

1. revoke or rotate the secret immediately;
2. delete the private repository if no useful review history must be retained;
3. remove the value from the source and rebuild clean history;
4. rerun both source and history secret scans;
5. create a new private repository and repeat the review.

If a release workflow fails before creating a release, fix the cause on `main`
and publish a new version tag. Do not force-move the failed tag after anyone
could have fetched it.

If the repository is accidentally made public too early, change it back to
private immediately, but still treat every included credential as disclosed.
