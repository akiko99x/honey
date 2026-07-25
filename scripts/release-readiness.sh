#!/usr/bin/env bash
# One entry point for source/release readiness. It never publishes anything.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="static"
ALLOW_NO_GIT=false

usage() {
	cat <<'EOF'
usage: scripts/release-readiness.sh [--static|--full|--package] [--allow-no-git]

  --static       metadata, repository hygiene, migrations, UI/shell syntax
  --full         static + Go/Rust tests, feature matrix and protobuf drift
  --package      full + Linux release builds, archive/checksum inspection
  --allow-no-git run source checks before the owner initializes the repository
EOF
}

for arg in "$@"; do
	case "$arg" in
	--static) MODE="static" ;;
	--full) MODE="full" ;;
	--package) MODE="package" ;;
	--allow-no-git) ALLOW_NO_GIT=true ;;
	-h|--help) usage; exit 0 ;;
	*) echo "unknown argument: $arg" >&2; usage >&2; exit 2 ;;
	esac
done

cd "$ROOT"
fail() { echo "release readiness failed: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null || fail "missing tool: $1"; }
step() { printf '\n==> %s\n' "$*"; }

for tool in grep sed find sort sha256sum node; do need "$tool"; done

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' master/Cargo.toml | head -1)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] || fail "invalid Cargo version: $version"
agent_version="$(sed -n 's/^const agentVersion = "\([^"]*\)"/\1/p' agent/internal/grpcserver/server.go)"
[[ "$agent_version" == "$version" ]] || fail "master version $version != agent version $agent_version"
openapi_version="$(
	sed -n 's/.*"version":[[:space:]]*"\([^"]*\)".*/\1/p' web/openapi.json |
		head -1
)"
[[ "$openapi_version" == "$version" ]] ||
	fail "master version $version != OpenAPI version $openapi_version"

step "repository and license"
if [[ "$ALLOW_NO_GIT" == true ]]; then
	echo "git baseline intentionally skipped (repository initialization is owner-only)"
else
	need git
	git rev-parse --is-inside-work-tree >/dev/null 2>&1 || fail "not a Git work tree"
	[[ -z "$(git status --porcelain)" ]] || fail "Git work tree is not clean"
	tracked="$(mktemp)"
	trap 'rm -f "$tracked"' EXIT
	git ls-files >"$tracked"
	if grep -E '(^|/)(build|dist|coverage|backups)/|(^|/)(credentials\.env|\.smoke-|[^/]*\.(key|pem|p12|pfx|db|sqlite|sqlite3|log))$' "$tracked"; then
		fail "runtime/secret-shaped files are tracked"
	fi
fi
grep -q 'GNU AFFERO GENERAL PUBLIC LICENSE' LICENSE || fail "LICENSE is not canonical AGPL text"
grep -q 'license = "AGPL-3.0-only"' master/Cargo.toml || fail "Cargo license metadata mismatch"
grep -q 'AGPL-3.0-only' README.md || fail "README license declaration missing"
for file in \
	README.md CHANGELOG.md SECURITY.md SUPPORT.md CONTRIBUTING.md LICENSE \
	docs/handbook.md docs/error-codes.md docs/release-checklist.md \
	docs/github-publication.md docs/docker-deployment.md \
	deploy/docker/compose.yml deploy/docker/Caddyfile deploy/docker/README.md \
	.github/PULL_REQUEST_TEMPLATE.md \
	.github/ISSUE_TEMPLATE/bug_report.yml .github/dependabot.yml; do
	[[ -s "$file" ]] || fail "required release file missing: $file"
done
echo "metadata: v$version, AGPL-3.0-only"

step "migration order"
previous=0
first=1
while IFS= read -r migration; do
	number="$(basename "$migration" | sed 's/_.*//')"
	value=$((10#$number))
	[[ "$value" -gt "$previous" ]] || fail "migration order is not strictly increasing at $number"
	if [[ "$first" -eq 1 && "$value" -ne 1 ]]; then
		fail "first migration must be 0001, got $number"
	fi
	previous="$value"
	first=0
done < <(find master/migrations -maxdepth 1 -type f -name '[0-9][0-9][0-9][0-9]_*.sql' | sort)
[[ "$first" -eq 0 ]] || fail "no migrations found"
echo "migrations: 0001..$(printf '%04d' "$previous") (ordered; gaps allowed)"

step "static syntax"
node --check web/app.js
bash -n scripts/*.sh
bash -n deploy/docker/*.sh
grep -Fq '${HONEY_POSTGRES_BIND:-127.0.0.1}' deploy/docker/compose.yml ||
	fail "Docker PostgreSQL must bind to loopback by default"
grep -Fq 'master_config:/etc/honey' deploy/docker/compose.yml ||
	fail "Docker master must use a dedicated config volume"
grep -Fq 'agent_config:/etc/honey' deploy/docker/compose.yml ||
	fail "Docker agent must use a dedicated config volume"
if grep -Eq '/var/run/docker\.sock|/run/docker\.sock' deploy/docker/compose.yml; then
	fail "Docker socket must not be mounted into honey services"
fi
if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
	HONEY_PANEL_DOMAIN=panel.example.com \
		docker compose -f deploy/docker/compose.yml config --quiet
else
	echo "Docker Compose syntax check skipped (docker compose unavailable)"
fi
echo "UI and shell syntax: ok"

step "bootstrap sandbox contract"
grep -Eq '^ReadWritePaths=.* /etc/honey/master-certs( |$)' \
	deploy/systemd/honey-master.service ||
	fail "master service cannot write its enrollment PKI directory"
if grep -Fq 'Host: $panel_domain' scripts/bootstrap.sh; then
	fail "local bootstrap API calls must keep their loopback Host"
fi
grep -Fq 'caddy fmt --overwrite /etc/caddy/Caddyfile' scripts/bootstrap.sh ||
	fail "bootstrap must format the generated Caddyfile"
grep -Fq "printf 'http://%s {\\n' \"\$panel_domain\"" scripts/bootstrap.sh ||
	fail "bootstrap must bind the ACME HTTP handler to the panel domain"
grep -Fq "printf 'https://%s {\\n' \"\$panel_domain\"" scripts/bootstrap.sh ||
	fail "bootstrap must bind the HTTPS panel handler to the panel domain"
echo "bootstrap Caddy, session and PKI sandbox contract: ok"

if [[ "$MODE" == "static" ]]; then
	echo "release readiness (static): ok"
	exit 0
fi

for tool in go cargo; do need "$tool"; done

step "Go and protobuf"
(cd agent && go test ./...)
before="$(mktemp)"
after="$(mktemp)"
trap 'rm -f "$before" "$after"' EXIT
sha256sum agent/gen/honey/v1/*.go | sort >"$before"
(cd agent && go run github.com/bufbuild/buf/cmd/buf@v1.47.2 generate ../proto)
sha256sum agent/gen/honey/v1/*.go | sort >"$after"
cmp -s "$before" "$after" || fail "generated protobuf files drifted"

step "Rust test and feature matrix"
(cd master && cargo fmt -- --check)
(cd master && cargo test --locked)
(cd master && cargo test --locked --features dial-acceptor)
(cd master && cargo check --locked --features tls)
(cd master && cargo check --locked --features acme)
(cd master && cargo check --locked --features dial-acceptor,acme)

if [[ "$MODE" == "full" ]]; then
	echo "release readiness (full): ok"
	exit 0
fi

[[ "$(uname -s)" == Linux ]] || fail "--package requires Linux"
for tool in tar go cargo; do need "$tool"; done

step "Linux package and checksum"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cargo build --locked --release --manifest-path master/Cargo.toml --features dial-acceptor,acme
(cd agent && CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -ldflags='-s -w' -o "$work/honey-agent" ./cmd/agent)
(cd agent && CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath -ldflags='-s -w' -o "$work/honey-enroll" ./cmd/enroll)
bash scripts/package-release.sh "v$version" \
	master/target/release/honey-master "$work/honey-agent" "$work/honey-enroll" "$work/dist"
archive="$work/dist/honey-${version}-linux-amd64.tar.gz"
(cd "$work/dist" && sha256sum -c "$(basename "$archive").sha256")
contents="$(tar -tzf "$archive")"
for required in \
	bin/honey-master bin/honey-agent bin/honey-enroll \
	scripts/install.sh scripts/install-release.sh scripts/bootstrap.sh \
	scripts/install-docker.sh scripts/docker-backup.sh scripts/docker-restore-check.sh \
	README.md LICENSE deploy/systemd/honey-master.service \
	deploy/docker/compose.yml deploy/docker/Caddyfile; do
	grep -q "honey-${version}-linux-amd64/${required}$" <<<"$contents" || fail "archive missing $required"
done
if grep -E '\.(key|pem|p12|pfx|db|sqlite|log|env)$' <<<"$contents"; then
	fail "archive contains a runtime/secret-shaped file"
fi
echo "release readiness (package): ok — $archive"
