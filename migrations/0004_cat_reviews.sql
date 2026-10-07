-- Cat reviews: a free-text note plus a 0-10 rating, one per user per cat.
CREATE TABLE cat_reviews (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    cat_id     uuid NOT NULL REFERENCES cats(id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body       text NOT NULL,
    rating     smallint NOT NULL CHECK (rating BETWEEN 0 AND 10),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (cat_id, user_id)
);
CREATE INDEX cat_reviews_cat_id_idx ON cat_reviews (cat_id);

-- Likes on a review. Unlike names, a user may like many reviews — no uniqueness cap.
CREATE TABLE review_likes (
    review_id  uuid NOT NULL REFERENCES cat_reviews(id) ON DELETE CASCADE,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (review_id, user_id)
);
