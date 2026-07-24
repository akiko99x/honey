#!/usr/bin/env bash
# Download, verify and install a published honey GitHub release.
set -euo pipefail

usage() {
	cat >&2 <<'EOF'
usage: install-release.sh [--repo owner/repo] [--version latest|vX.Y.Z] [--enable] [--start]
EOF
	exit 2
}

[[ "${EUID}" -eq 0 ]] || { echo "run as root" >&2; exit 1; }
for command in curl tar sha256sum sed; do
	command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done

repo="${HONEY_INSTALL_REPO:-akiko99x/honey}"
requested="latest"
start=0
enable=0
while (($#)); do
	case "$1" in
		--repo) (($# >= 2)) || usage; repo="$2"; shift 2 ;;
		--version) (($# >= 2)) || usage; requested="$2"; shift 2 ;;
		--enable) enable=1; shift ;;
		--start) start=1; enable=1; shift ;;
		--help|-h) usage ;;
		*) usage ;;
	esac
done
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] ||
	{ echo "invalid GitHub repository: $repo" >&2; exit 2; }

case "$(uname -m)" in
	x86_64|amd64) platform="linux-amd64" ;;
	*) echo "no published honey release for architecture $(uname -m)" >&2; exit 1 ;;
esac

if [[ "$requested" == "latest" ]]; then
	tag="$(
		curl -fsSL -H 'Accept: application/vnd.github+json' \
			"https://api.github.com/repos/$repo/releases/latest" |
			sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' |
			head -n 1
	)"
	[[ -n "$tag" ]] || { echo "could not determine latest release tag" >&2; exit 1; }
else
	tag="$requested"
fi
[[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] ||
	{ echo "invalid release tag: $tag" >&2; exit 2; }

version="${tag#v}"
asset="honey-${version}-${platform}.tar.gz"
base="https://github.com/$repo/releases/download/$tag"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$tmp/$asset" "$base/$asset"
curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$tmp/$asset.sha256" "$base/$asset.sha256"
(
	cd "$tmp"
	sha256sum -c "$asset.sha256"
)

tar -xzf "$tmp/$asset" -C "$tmp"
root="$tmp/honey-${version}-${platform}"
[[ -x "$root/scripts/install.sh" ]] || { echo "release installer missing from archive" >&2; exit 1; }

args=(--update-repo "$repo" "$tmp/$asset")
((enable)) && args=(--enable --update-repo "$repo" "$tmp/$asset")
((start)) && args=(--enable --start --update-repo "$repo" "$tmp/$asset")
"$root/scripts/install.sh" "${args[@]}"
echo "installed honey $tag from $repo"
