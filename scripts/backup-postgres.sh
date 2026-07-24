#!/usr/bin/env bash
# Create an atomic, checksummed PostgreSQL backup, optionally encrypt it with
# GPG, and retain only the newest HONEY_BACKUP_KEEP artifacts.
set -euo pipefail
umask 077

: "${DATABASE_URL:?set DATABASE_URL}"
OUT="${1:-./backups}"
KEEP="${HONEY_BACKUP_KEEP:-14}"
RECIPIENT="${HONEY_BACKUP_GPG_RECIPIENT:-}"

if [[ ! "$KEEP" =~ ^[1-9][0-9]*$ ]]; then
	echo "HONEY_BACKUP_KEEP must be a positive integer" >&2
	exit 2
fi

mkdir -p "$OUT"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
file="${OUT}/honey-${stamp}.dump"
tmp_dump="$(mktemp "${OUT}/.honey-${stamp}.XXXXXX.dump")"
tmp_encrypted=""

cleanup() {
	rm -f "$tmp_dump"
	[[ -z "$tmp_encrypted" ]] || rm -f "$tmp_encrypted"
}
trap cleanup EXIT

pg_dump --format=custom --no-owner --no-acl --file="$tmp_dump" "$DATABASE_URL"

if [[ -n "$RECIPIENT" ]]; then
	command -v gpg >/dev/null 2>&1 || {
		echo "gpg is required when HONEY_BACKUP_GPG_RECIPIENT is set" >&2
		exit 1
	}
	file="${file}.gpg"
	tmp_encrypted="$(mktemp "${OUT}/.honey-${stamp}.XXXXXX.dump.gpg")"
	gpg --batch --yes --trust-model always \
		--recipient "$RECIPIENT" \
		--output "$tmp_encrypted" \
		--encrypt "$tmp_dump"
	mv -f "$tmp_encrypted" "$file"
	tmp_encrypted=""
else
	mv -f "$tmp_dump" "$file"
fi

(
	cd "$OUT"
	sha256sum "$(basename "$file")" >"$(basename "$file").sha256.tmp"
)
mv -f "${file}.sha256.tmp" "${file}.sha256"

mapfile -t artifacts < <(
	find "$OUT" -maxdepth 1 -type f \
		\( -name 'honey-*.dump' -o -name 'honey-*.dump.gpg' \) \
		-printf '%f\n' | sort -r
)
for ((index = KEEP; index < ${#artifacts[@]}; index++)); do
	old="${OUT}/${artifacts[$index]}"
	rm -f -- "$old" "${old}.sha256"
done

trap - EXIT
cleanup
echo "$file"
