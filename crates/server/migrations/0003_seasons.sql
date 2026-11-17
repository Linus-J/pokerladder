-- Phase 4: Glicko-2 ratings and monthly seasons.
--
-- users.rating is now a long-running Glicko-2 rating used only for matchmaking (so
-- pairings stay sensible at the start of a season). What players see and compete on
-- is their season rating: a fresh Glicko-2 rating for each calendar month (UTC).

-- The current season: the calendar month in UTC, as 'YYYY-MM'.
CREATE FUNCTION current_season() RETURNS TEXT
    LANGUAGE sql STABLE
    AS $$ SELECT to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM') $$;

-- Beta ratings were Elo; everyone restarts as a new Glicko-2 player (the About page
-- warned that beta ratings might be reset). Match records keep their Elo history.
ALTER TABLE users
    ADD COLUMN rd DOUBLE PRECISION NOT NULL DEFAULT 350,
    ADD COLUMN volatility DOUBLE PRECISION NOT NULL DEFAULT 0.06,
    -- When this rating last changed (for ageing its deviation); NULL if never.
    ADD COLUMN rated_at TIMESTAMPTZ;
UPDATE users SET rating = 1500;

CREATE TABLE season_ratings (
    season     TEXT NOT NULL CHECK (season ~ '^[0-9]{4}-(0[1-9]|1[0-2])$'),
    user_id    BIGINT NOT NULL REFERENCES users ON DELETE CASCADE,
    rating     DOUBLE PRECISION NOT NULL,
    rd         DOUBLE PRECISION NOT NULL,
    volatility DOUBLE PRECISION NOT NULL,
    matches    INTEGER NOT NULL DEFAULT 0,
    wins       INTEGER NOT NULL DEFAULT 0,
    rated_at   TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (season, user_id)
);
CREATE INDEX season_ratings_user_idx ON season_ratings (user_id, season);

-- Whether a match counts. Decided when it is created: a pair's 4th match within 24
-- hours (and any after) is unrated, so friends can't farm rating off each other.
-- For rated matches finished from now on, rating_before*/rating_after* hold the
-- players' season ratings, and `season` the season they counted towards.
ALTER TABLE matches
    ADD COLUMN rated BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN season TEXT;
CREATE INDEX matches_pair_idx ON matches (LEAST(player0, player1), GREATEST(player0, player1), started_at DESC);
