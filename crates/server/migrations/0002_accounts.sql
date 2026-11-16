-- A name requested in a sign-up whose email already had an account. Holding it for a
-- while makes "name taken" answers the same whether or not the email was registered,
-- so a second sign-up can't probe which emails have accounts.
CREATE TABLE name_reservations (
    name_lower TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL
);

-- Password-reset tokens (SHA-256 hashes only, like sessions).
CREATE TABLE password_resets (
    token_hash BYTEA PRIMARY KEY,
    user_id    BIGINT NOT NULL REFERENCES users ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX password_resets_user_idx ON password_resets (user_id);
