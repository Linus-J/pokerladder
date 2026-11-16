#!/usr/bin/env bash
# Dumps the production database to ~/backups/pokerladder and keeps the newest 14.
# Copy that directory somewhere off this machine too (see README.md).
set -euo pipefail

dir="${BACKUP_DIR:-$HOME/backups/pokerladder}"
keep="${BACKUP_KEEP:-14}"
mkdir -p "$dir"
chmod 700 "$dir"

out="$dir/pokerladder-$(date -u +%Y%m%dT%H%M%SZ).dump"
docker exec pokerladder-prod-db-1 pg_dump -U pokerladder -d pokerladder -Fc >"$out.tmp"
# An empty or truncated dump must never replace a good one.
test -s "$out.tmp"
mv "$out.tmp" "$out"
chmod 600 "$out"

ls -1t "$dir"/pokerladder-*.dump | tail -n +"$((keep + 1))" | xargs -r rm --
echo "backup written: $out ($(du -h "$out" | cut -f1))"
