#!/usr/bin/env bash
# Download exactly one GitHub release asset and require GitHub's sha256 digest.
set -euo pipefail

repo="${1:?usage: fetch-github-release-asset.sh owner/repo tag|latest regex output-dir}"
requested="${2:?missing release tag}"
pattern="${3:?missing asset regex}"
output_dir="${4:?missing output directory}"

[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || {
	echo "invalid GitHub repository: $repo" >&2
	exit 2
}

if [[ "$requested" == "latest" ]]; then
	endpoint="https://api.github.com/repos/$repo/releases/latest"
else
	tag="$requested"
	[[ "$tag" == v* || "$tag" == */v* ]] || tag="v$tag"
	encoded_tag="$(python3 - "$tag" <<'PY'
import sys
import urllib.parse

print(urllib.parse.quote(sys.argv[1], safe=""))
PY
)"
	endpoint="https://api.github.com/repos/$repo/releases/tags/$encoded_tag"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl_args=(-fsSL --retry 3 --proto '=https' --tlsv1.2 -H 'Accept: application/vnd.github+json')
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
	curl_args+=(-H "Authorization: Bearer $GITHUB_TOKEN")
fi
curl "${curl_args[@]}" -o "$tmp/release.json" "$endpoint"

mapfile -t meta < <(
	python3 - "$tmp/release.json" "$pattern" <<'PY'
import json
import re
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    release = json.load(handle)
matches = [
    asset for asset in release.get("assets", [])
    if re.fullmatch(sys.argv[2], asset.get("name", ""))
]
if len(matches) != 1:
    raise SystemExit(
        f"expected one asset matching {sys.argv[2]!r}, found {len(matches)}"
    )
asset = matches[0]
print(asset["name"])
print(asset["browser_download_url"])
print(asset.get("digest") or "")
PY
)

((${#meta[@]} == 3)) || {
	echo "could not resolve release asset for $repo" >&2
	exit 1
}
name="${meta[0]}"
url="${meta[1]}"
digest="${meta[2]#sha256:}"
[[ "$digest" =~ ^[0-9a-fA-F]{64}$ ]] || {
	echo "$repo does not publish a sha256 digest for $name" >&2
	exit 1
}

install -d -m 0700 "$output_dir"
curl "${curl_args[@]}" -o "$output_dir/$name" "$url"
(
	cd "$output_dir"
	printf '%s  %s\n' "$digest" "$name" | sha256sum -c - >&2
)
printf '%s\n' "$output_dir/$name"
