#!/usr/bin/env bash
# Wipes the ladder back to a clean start, keeping accounts: every match, hand and
# hand history, every season's standings, and everyone's ratings (back to 1500).
# Logins, sessions and email verification are untouched, so nobody has to sign up
# again. For testing the rating system from scratch; not something to do once
# prizes are running.
#
# Refuses while a match is in progress (deploy or wait first). Takes a backup
# before wiping (restore it as described in README.md if you change your mind).
#
#   deploy/home/reset-ladder.sh          asks you to type "reset"
#   deploy/home/reset-ladder.sh --yes    no question (for scripts)
#
# DB_CONTAINER picks another database container, e.g. the dev one:
#   DB_CONTAINER=pokerladder-db-1 deploy/home/reset-ladder.sh
# (no backup is taken for anything but the production container).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
container="${DB_CONTAINER:-pokerladder-prod-db-1}"
# No -i except where SQL is piped in: it would swallow the answer typed below.
psql=(docker exec "$container" psql -U pokerladder -d pokerladder -v ON_ERROR_STOP=1 -qtA)

counts() {
  "${psql[@]}" -c "SELECT format('%s users, %s matches (%s in progress), %s hands, %s season rows',
      (SELECT count(*) FROM users), (SELECT count(*) FROM matches),
      (SELECT count(*) FROM matches WHERE status = 'in_progress'),
      (SELECT count(*) FROM hands), (SELECT count(*) FROM season_ratings))"
}

echo "$container: $(counts)"
if [[ "${1:-}" != "--yes" ]]; then
  read -r -p 'Wipe all matches, hands and ratings (accounts stay)? Type "reset": ' answer
  [[ "$answer" == "reset" ]] || { echo "nothing changed"; exit 1; }
fi

if [[ "$container" == "pokerladder-prod-db-1" ]]; then
  "$here/backup.sh"
else
  echo "not the production container: no backup taken"
fi

# One transaction. Locking `matches` first means no match can start or finish
# while this runs; a match in progress aborts the whole thing (nothing changes).
docker exec -i "$container" psql -U pokerladder -d pokerladder -v ON_ERROR_STOP=1 -qtA <<'SQL'
BEGIN;
-- Give up rather than queue the live site behind a long wait for the lock.
SET LOCAL lock_timeout = '5s';
LOCK TABLE matches IN ACCESS EXCLUSIVE MODE;
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM matches WHERE status = 'in_progress') THEN
    RAISE EXCEPTION 'a match is in progress: wait for it to finish (nothing was changed)';
  END IF;
END $$;
-- Ids keep counting (no RESTART IDENTITY): a match id is never reused, so nothing
-- still holding an old one can write into a new match.
TRUNCATE hands, matches, season_ratings;
UPDATE users SET rating = 1500, rd = 350, volatility = 0.06, rated_at = NULL, rated_matches = 0;
COMMIT;
SQL

echo "done: $(counts)"
