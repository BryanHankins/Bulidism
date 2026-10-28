-- Email verification. Accounts created before this migration count as verified, so nobody is locked
-- out of sharing by the upgrade.
ALTER TABLE users ADD COLUMN email_verified_at timestamptz;
UPDATE users SET email_verified_at = created_at;

-- Single-use verification tokens; token_hash = sha256 of the token mailed to the user.
CREATE TABLE email_verifications (
    token_hash bytea PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX email_verifications_user_idx ON email_verifications(user_id);
CREATE INDEX email_verifications_expires_idx ON email_verifications(expires_at);
