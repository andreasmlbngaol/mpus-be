-- A user may like at most one name per cat: your like follows the name you back.
-- Enforced at the DB level so no code path can drift from the rule.

-- Likes now carry their cat, so (user_id, cat_id) can be unique.
ALTER TABLE name_likes ADD COLUMN cat_id uuid REFERENCES cats(id) ON DELETE CASCADE;
UPDATE name_likes l SET cat_id = n.cat_id FROM cat_names n WHERE n.id = l.name_id;
ALTER TABLE name_likes ALTER COLUMN cat_id SET NOT NULL;

-- Collapse any pre-existing multi-likes down to the most recent one per (user, cat).
WITH ranked AS (
    SELECT l.name_id, l.user_id,
           row_number() OVER (
               PARTITION BY l.user_id, l.cat_id
               ORDER BY l.created_at DESC, l.name_id
           ) AS rn
    FROM name_likes l
)
DELETE FROM name_likes l
USING ranked r
WHERE l.name_id = r.name_id AND l.user_id = r.user_id AND r.rn > 1;

ALTER TABLE name_likes ADD CONSTRAINT name_likes_one_per_cat UNIQUE (user_id, cat_id);
CREATE INDEX name_likes_cat_id_idx ON name_likes (cat_id);
