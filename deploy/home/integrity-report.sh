#!/usr/bin/env bash
# Prints the chip-dumping report for a season (read-only; see
# crates/server/src/integrity/mod.rs for what each signal means). Review flagged
# pairs before any payout: they are leads, not proof.
#
#   deploy/home/integrity-report.sh                    current (UTC) month
#   deploy/home/integrity-report.sh --season 2026-11   a given season
#   deploy/home/integrity-report.sh --all              also list unflagged pairs
#
# Uses the production DATABASE_URL from pokerladder.env (never printed) and the
# binary `update.sh` builds. Read a cited hand with:
#   docker exec -it pokerladder-prod-db-1 psql -U pokerladder \
#     -c "SELECT history_audit FROM hands WHERE id = <hand>"
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
env_file="${POKERLADDER_ENV:-$HOME/.config/pokerladder/pokerladder.env}"
bin="$repo/target/release/integrity"

[[ -x "$bin" ]] || { echo "no $bin: run cargo build --release -p pokerladder-server" >&2; exit 1; }
DATABASE_URL="$(grep -m1 '^DATABASE_URL=' "$env_file" | cut -d= -f2- || true)"
[[ -n "$DATABASE_URL" ]] || { echo "DATABASE_URL not found in $env_file" >&2; exit 1; }
export DATABASE_URL
exec "$bin" "$@"
