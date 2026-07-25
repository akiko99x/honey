#!/usr/bin/env bash
# Build a reproducible Linux release archive from already-built binaries.
set -euo pipefail

version="${1:?usage: package-release.sh <version> <honey-master> <honey-agent> <honey-enroll> [out-dir]}"
master_bin="${2:?missing honey-master binary}"
agent_bin="${3:?missing honey-agent binary}"
enroll_bin="${4:?missing honey-enroll binary}"
out="${5:-dist}"

[[ "$version" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] || {
	echo "invalid release version: $version" >&2
	exit 2
}
version="${version#v}"

for binary in "$master_bin" "$agent_bin" "$enroll_bin"; do
	[[ -f "$binary" ]] || {
		echo "binary not found: $binary" >&2
		exit 1
	}
done

mkdir -p "$out"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
root="$stage/honey-${version}-linux-amd64"
install -d "$root/bin" "$root/deploy/systemd" "$root/deploy/docker" "$root/scripts" "$root/docs"
install -m 0755 "$master_bin" "$root/bin/honey-master"
install -m 0755 "$agent_bin" "$root/bin/honey-agent"
install -m 0755 "$enroll_bin" "$root/bin/honey-enroll"
install -m 0755 scripts/install.sh scripts/install-release.sh scripts/bootstrap.sh \
	scripts/install-docker.sh scripts/docker-backup.sh scripts/docker-restore-check.sh \
	scripts/fetch-github-release-asset.sh scripts/gen-certs.sh \
	scripts/backup-postgres.sh scripts/restore-check.sh "$root/scripts/"
cp -a deploy/systemd/. "$root/deploy/systemd/"
cp -a deploy/docker/. "$root/deploy/docker/"
cp -a docs/. "$root/docs/"
cp README.md SECURITY.md CONTRIBUTING.md LICENSE "$root/"

archive="$out/honey-${version}-linux-amd64.tar.gz"
tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
	-C "$stage" -czf "$archive" "$(basename "$root")"
(
	cd "$out"
	sha256sum "$(basename "$archive")" >"$(basename "$archive").sha256"
)
printf '%s\n' "$archive" "${archive}.sha256"
