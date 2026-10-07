-- Moderation, cat merge, and notifications.

-- Moderation: a user can report a name or review. Content hides itself once enough
-- *different* people report it, so no human has to be on call. Nothing is deleted —
-- `hidden` just drops it from listings, and a false alarm is one UPDATE away from back.
ALTER TABLE cat_names ADD COLUMN hidden boolean NOT NULL DEFAULT false;
ALTER TABLE cat_reviews ADD COLUMN hidden boolean NOT NULL DEFAULT false;

CREATE TABLE reports (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    reporter_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_kind text NOT NULL CHECK (target_kind IN ('name', 'review')),
    target_id   uuid NOT NULL,
    reason      text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    -- One report per user per item: re-tapping does nothing, and a mob can't
    -- stack a single account's weight.
    UNIQUE (reporter_id, target_kind, target_id)
);
CREATE INDEX reports_target_idx ON reports (target_kind, target_id);

-- Merge: two cat records that are really the same cat. Both owners must agree, so
-- nobody can dissolve a cat someone else built. `merged_into` keeps old links alive —
-- a cat that lost a merge redirects instead of 404ing.
ALTER TABLE cats ADD COLUMN merged_into uuid REFERENCES cats(id);

CREATE TABLE merge_requests (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source_cat_id   uuid NOT NULL REFERENCES cats(id) ON DELETE CASCADE,
    target_cat_id   uuid NOT NULL REFERENCES cats(id) ON DELETE CASCADE,
    requested_by    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source_approved boolean NOT NULL DEFAULT false,
    target_approved boolean NOT NULL DEFAULT false,
    status          text NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'merged', 'rejected', 'undone')),
    created_at      timestamptz NOT NULL DEFAULT now(),
    resolved_at     timestamptz,
    CHECK (source_cat_id <> target_cat_id)
);
CREATE INDEX merge_requests_source_idx ON merge_requests (source_cat_id, status);
CREATE INDEX merge_requests_target_idx ON merge_requests (target_cat_id, status);

-- What a merge actually moved, so it can be undone: every row that was re-pointed from
-- the source cat to the target. Undo walks this and points them back. Name-likes aren't
-- tracked separately — they follow their name, which is tracked here.
CREATE TABLE merge_moves (
    merge_id    uuid NOT NULL REFERENCES merge_requests(id) ON DELETE CASCADE,
    kind        text NOT NULL CHECK (kind IN ('sighting', 'name', 'review')),
    moved_id    uuid NOT NULL,
    PRIMARY KEY (merge_id, kind, moved_id)
);

-- Notifications: in-app inbox. No push provider needed — the app polls the unread
-- count while it's open. Each row points at the actor and the cat so the client can
-- deep-link without a second lookup.
CREATE TABLE notifications (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    actor_id   uuid REFERENCES users(id) ON DELETE CASCADE,
    kind       text NOT NULL CHECK (kind IN ('name_liked', 'name_added', 'review_added', 'merge_requested', 'merge_resolved')),
    cat_id     uuid REFERENCES cats(id) ON DELETE CASCADE,
    seen_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX notifications_user_idx ON notifications (user_id, created_at DESC);
