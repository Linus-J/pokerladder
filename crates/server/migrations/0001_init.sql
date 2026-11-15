-- Accounts. Names and emails are unique case-insensitively.
CREATE TABLE users (
    id                BIGSERIAL PRIMARY KEY,
    name              TEXT NOT NULL CHECK (name ~ '^[A-Za-z0-9_-]{1,20}$'),
    email             TEXT NOT NULL CHECK (length(email) <= 254),
    password_hash     TEXT NOT NULL,
    email_verified_at TIMESTAMPTZ,
    rating            DOUBLE PRECISION NOT NULL DEFAULT 1500,
    rated_matches     INTEGER NOT NULL DEFAULT 0,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX users_name_key ON users (lower(name));
CREATE UNIQUE INDEX users_email_key ON users (lower(email));
CREATE INDEX users_rating_idx ON users (rating DESC);

-- Only SHA-256 hashes of tokens are stored, so a database leak doesn't leak sessions.
CREATE TABLE sessions (
    token_hash BYTEA PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_user_idx ON sessions (user_id);

CREATE TABLE email_verifications (
    token_hash BYTEA PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX email_verifications_user_idx ON email_verifications (user_id);

-- One heads-up match. Ratings are recorded before/after in the same transaction that
-- finishes the match and updates users.rating.
CREATE TABLE matches (
    id             BIGSERIAL PRIMARY KEY,
    player0        BIGINT NOT NULL REFERENCES users,
    player1        BIGINT NOT NULL REFERENCES users,
    status         TEXT NOT NULL DEFAULT 'in_progress'
                   CHECK (status IN ('in_progress', 'finished', 'abandoned')),
    winner_seat    SMALLINT CHECK (winner_seat IN (0, 1)),
    hands_played   INTEGER,
    rating_before0 DOUBLE PRECISION,
    rating_before1 DOUBLE PRECISION,
    rating_after0  DOUBLE PRECISION,
    rating_after1  DOUBLE PRECISION,
    started_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at    TIMESTAMPTZ,
    CHECK (player0 <> player1),
    CHECK ((status = 'finished') = (winner_seat IS NOT NULL))
);
CREATE INDEX matches_player0_idx ON matches (player0, started_at DESC);
CREATE INDEX matches_player1_idx ON matches (player1, started_at DESC);

-- One hand. `audit_events` is the UNFILTERED event stream (every hole card, including
-- folded and mucked hands) for integrity review; it is never sent to clients. The
-- histories are rendered when the hand ends, one per viewer.
CREATE TABLE hands (
    id            BIGSERIAL PRIMARY KEY,
    match_id      BIGINT NOT NULL REFERENCES matches ON DELETE CASCADE,
    number        INTEGER NOT NULL,
    started_at    TIMESTAMPTZ NOT NULL,
    audit_events  JSONB NOT NULL,
    history_seat0 TEXT NOT NULL,
    history_seat1 TEXT NOT NULL,
    history_audit TEXT NOT NULL,
    UNIQUE (match_id, number)
);
