-- Device push tokens for Firebase Cloud Messaging.
--
-- One row per device install. A token belongs to exactly one user at a time (the last
-- to log in on that device claims it), so the token is the primary key and `user_id`
-- moves on re-registration. Logout deletes the row so a signed-out device stops getting
-- that account's pushes.
CREATE TABLE device_tokens (
    token      text PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    platform   text NOT NULL DEFAULT 'android',
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX device_tokens_user_idx ON device_tokens (user_id);
