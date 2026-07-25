#!/usr/bin/env bash
set -euo pipefail

install_dir="${HONEY_DOCKER_DIR:-/opt/honey-docker}"
[[ -f "$install_dir/compose.yml" ]] || {
	echo "Docker deployment not found at $install_dir" >&2
	exit 1
}
cd "$install_dir"
exec docker compose run --rm --no-deps backup backup
