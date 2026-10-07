-- MPUS initial schema.
-- email & username are plain text (not citext) to keep sqlx simple.
-- The app lowercases them before every insert/compare.

CREATE TABLE users (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email             text NOT NULL UNIQUE,
    email_verified_at timestamptz,
    password_hash     text NOT NULL,
    username          text NOT NULL UNIQUE,
    nickname          text NOT NULL,
    avatar_url        text,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

-- Sessions: opaque tokens, stored as a sha256 hash (never plaintext).
CREATE TABLE sessions (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash   text NOT NULL UNIQUE,
    expires_at   timestamptz NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz NOT NULL DEFAULT now(),
    user_agent   text
);
CREATE INDEX sessions_user_id_idx ON sessions (user_id);

-- Email verification & password reset tokens: single-use, short-lived, stored hashed.
CREATE TABLE email_tokens (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind       text NOT NULL CHECK (kind IN ('verify', 'reset')),
    token_hash text NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    used_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX email_tokens_user_id_idx ON email_tokens (user_id);
