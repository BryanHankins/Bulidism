-- Single-use password-reset tokens; token_hash = sha256 of the token mailed to the user.
CREATE TABLE password_resets (
    token_hash bytea PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX password_resets_user_idx ON password_resets(user_id);
CREATE INDEX password_resets_expires_idx ON password_resets(expires_at);
