#!/usr/bin/env bash
set -euo pipefail

file="${1:?usage: docker-restore-check.sh <backup filename>}"
[[ "$file" == "$(basename "$file")" ]] || {
	echo "pass a backup filename from the Docker backup directory" >&2
	exit 2
}
case "$file" in
honey-*.dump|honey-*.dump.gpg) ;;
*)
	echo "backup filename must end in .dump or .dump.gpg" >&2
	exit 2
	;;
esac
install_dir="${HONEY_DOCKER_DIR:-/opt/honey-docker}"
[[ -f "$install_dir/compose.yml" ]] || {
	echo "Docker deployment not found at $install_dir" >&2
	exit 1
}
[[ -f "$install_dir/backups/$file" ]] || {
	echo "backup not found: $install_dir/backups/$file" >&2
	exit 1
}
cd "$install_dir"
exec docker compose run --rm backup restore-check "/backups/$file"
