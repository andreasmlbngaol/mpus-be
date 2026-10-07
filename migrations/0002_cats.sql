-- Cat identification: cats, sightings, names and likes.
-- Requires the `vector` extension (pgvector). On the VM it is already installed;
-- locally: `sudo apt install postgresql-16-pgvector`, then CREATE EXTENSION as a
-- superuser. The migration below attempts it too, and is a no-op if it exists.

CREATE EXTENSION IF NOT EXISTS vector;

-- A cat is an identity many sightings can point at.
CREATE TABLE cats (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    created_by   uuid REFERENCES users(id) ON DELETE SET NULL,
    display_name text,
    created_at   timestamptz NOT NULL DEFAULT now()
);

-- One photo of a cat. cat_id is NULL until the uploader resolves it
-- (link to an existing cat, or start a new one).
CREATE TABLE sightings (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cat_id     uuid REFERENCES cats(id) ON DELETE SET NULL,
    photo_url  text NOT NULL,
    thumb_url  text NOT NULL,
    embedding  vector(768) NOT NULL,
    lat        double precision NOT NULL,
    lng        double precision NOT NULL,
    taken_at   timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sightings_cat_id_idx    ON sightings (cat_id);
CREATE INDEX sightings_geo_idx       ON sightings (lat, lng);
CREATE INDEX sightings_embedding_idx ON sightings USING hnsw (embedding vector_cosine_ops);

-- One name per user per cat; everyone's names are visible to everyone.
CREATE TABLE cat_names (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    cat_id     uuid NOT NULL REFERENCES cats(id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (cat_id, user_id)
);
CREATE INDEX cat_names_cat_id_idx ON cat_names (cat_id);

-- Likes on a name: "I agree with this one".
CREATE TABLE name_likes (
    name_id    uuid NOT NULL REFERENCES cat_names(id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (name_id, user_id)
);
