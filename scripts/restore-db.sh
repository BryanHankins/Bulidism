#!/usr/bin/env bash
# Restore an encrypted backup made by .github/workflows/backup.yml.
#
#   1. Download a backup: GitHub > Actions > db-backup > pick a run > Artifacts
#      (or `gh run download <run-id>`), and unzip it: buildsim-db-<stamp>.dump.gpg + SHA256SUMS.
#   2. BACKUP_PASSPHRASE='...' scripts/restore-db.sh buildsim-db-<stamp>.dump.gpg 'postgres://...'
#
# The target may be the production database (Render > buildsim-db > External Database URL) or a
# fresh one to inspect first. Existing buildsim tables in the target are replaced, in a single
# transaction, after you type "yes" (or set RESTORE_YES=1). Needs gpg and pg_restore 17+; without a
# local pg_restore 17, set USE_DOCKER=1 to run it from the postgres:17 image.
set -euo pipefail
usage="usage: BACKUP_PASSPHRASE=... $0 <backup.dump.gpg> <target-database-url>"
file=${1:?$usage}
target=${2:?$usage}
: "${BACKUP_PASSPHRASE:?$usage}"

sums="$(dirname "$file")/SHA256SUMS"
if [ -f "$sums" ]; then
  (cd "$(dirname "$file")" && sha256sum --check --ignore-missing --quiet SHA256SUMS)
  echo "checksum ok"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
gpg --batch --quiet --pinentry-mode loopback --passphrase-file <(printf %s "$BACKUP_PASSPHRASE") \
  -o "$tmp/db.dump" --decrypt "$file"

restore() {
  if [ "${USE_DOCKER:-}" = 1 ]; then
    docker run --rm -i --network host -v "$tmp:/w" postgres:17 pg_restore "$@"
  else
    pg_restore "$@"
  fi
}
dump=$([ "${USE_DOCKER:-}" = 1 ] && echo /w/db.dump || echo "$tmp/db.dump")

echo "backup holds:"
restore --list "$dump" | grep 'TABLE DATA' | sed 's/.*TABLE DATA /  /'
masked=$(printf %s "$target" | sed -E 's#(://[^:/@]*):[^@]*@#\1:***@#')
if [ "${RESTORE_YES:-}" != 1 ]; then
  read -r -p "Replace the buildsim tables in $masked? Type yes: " answer
  [ "$answer" = yes ] || { echo "aborted"; exit 1; }
fi
restore --clean --if-exists --no-owner --no-privileges --single-transaction --dbname "$target" "$dump"
echo "restored into $masked"
