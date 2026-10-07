use uuid::Uuid;

use crate::{
    error::AppError,
    models::{
        cat::{CatMarker, Page},
        user::{User, UserProfile},
    },
    state::AppState,
};

/// Fetch a user by id, or `NotFound`.
pub async fn by_id(state: &AppState, id: Uuid) -> Result<User, AppError> {
    sqlx::query_as::<_, User>(
        "SELECT id, email, email_verified_at, password_hash, username, nickname,
                avatar_url, created_at, updated_at
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

/// A public profile: the user plus what they've contributed — their sighting/name
/// counts and the distinct cats they've photographed (keyset-paged, newest first).
pub async fn profile(
    state: &AppState,
    id: Uuid,
    limit: i64,
    cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)>,
) -> Result<UserProfile, AppError> {
    let user = by_id(state, id).await?;

    let (sightings_count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM sightings WHERE user_id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;

    let (names_count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM cat_names WHERE user_id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;

    let (cats_count,): (i64,) =
        sqlx::query_as("SELECT count(DISTINCT cat_id) FROM sightings WHERE user_id = $1 AND cat_id IS NOT NULL")
            .bind(id)
            .fetch_one(&state.db)
            .await?;

    let (after_ts, after_id) = match cursor {
        Some((ts, cid)) => (Some(ts), Some(cid)),
        None => (None, None),
    };

    // The cats this user has photographed, newest sighting first, one row per cat.
    // `(seen_at, id)` is the cat's newest sighting, so the cursor walks cats by recency.
    let mut rows = sqlx::query_as::<_, ProfileCat>(
        "SELECT * FROM (
             SELECT DISTINCT ON (c.id)
                    c.id, c.display_name, s.thumb_url, s.lat, s.lng,
                    (SELECT count(*) FROM sightings x WHERE x.cat_id = c.id) AS sighting_count,
                    s.created_at AS seen_at
             FROM cats c
             JOIN sightings s ON s.cat_id = c.id
             WHERE s.user_id = $1
             ORDER BY c.id, s.created_at DESC
         ) t
         WHERE ($2::timestamptz IS NULL OR (seen_at, id) < ($2, $3))
         ORDER BY seen_at DESC, id DESC
         LIMIT $4",
    )
    .bind(id)
    .bind(after_ts)
    .bind(after_id)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await?;

    let has_more = rows.len() as i64 > limit;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        rows.last().map(|r| crate::paging::encode(r.seen_at, r.id))
    } else {
        None
    };

    Ok(UserProfile {
        id: user.id,
        username: user.username,
        nickname: user.nickname,
        avatar_url: user.avatar_url,
        sightings_count,
        cats_count,
        names_count,
        cats: Page {
            items: rows.into_iter().map(ProfileCat::marker).collect(),
            next_cursor,
        },
    })
}

/// One row of the profile cats query: a map marker plus the recency key it pages on.
#[derive(sqlx::FromRow)]
struct ProfileCat {
    id: Uuid,
    display_name: Option<String>,
    thumb_url: String,
    lat: f64,
    lng: f64,
    sighting_count: i64,
    seen_at: chrono::DateTime<chrono::Utc>,
}

impl ProfileCat {
    fn marker(self) -> CatMarker {
        CatMarker {
            id: self.id,
            display_name: self.display_name,
            thumb_url: self.thumb_url,
            lat: self.lat,
            lng: self.lng,
            sighting_count: self.sighting_count,
        }
    }
}
