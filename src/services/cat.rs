use uuid::Uuid;

use crate::{
    error::AppError,
    models::cat::{Cat, CatDetail, CatMarker, NameView, ReviewView, SightingView},
    state::AppState,
};

/// Cats whose most recent sighting falls inside the map viewport.
pub async fn in_bbox(
    state: &AppState,
    min_lat: f64,
    min_lng: f64,
    max_lat: f64,
    max_lng: f64,
) -> Result<Vec<CatMarker>, AppError> {
    let rows = sqlx::query_as::<_, CatMarker>(
        "SELECT DISTINCT ON (c.id)
                c.id, c.display_name, s.thumb_url, s.lat, s.lng,
                (SELECT count(*) FROM sightings x WHERE x.cat_id = c.id) AS sighting_count
         FROM cats c
         JOIN sightings s ON s.cat_id = c.id
         WHERE c.merged_into IS NULL
           AND s.lat BETWEEN $1 AND $3 AND s.lng BETWEEN $2 AND $4
         ORDER BY c.id, s.created_at DESC",
    )
    .bind(min_lat)
    .bind(min_lng)
    .bind(max_lat)
    .bind(max_lng)
    .fetch_all(&state.db)
    .await?;
    Ok(rows)
}

/// Follow `merged_into` to the surviving cat. Old links keep working; a chain is
/// collapsed in one hop (merges never target an already-merged cat).
async fn resolve_redirect(state: &AppState, cat_id: Uuid) -> Result<Uuid, AppError> {
    let row: Option<(Option<Uuid>,)> =
        sqlx::query_as("SELECT merged_into FROM cats WHERE id = $1")
            .bind(cat_id)
            .fetch_optional(&state.db)
            .await?;
    match row {
        None => Err(AppError::NotFound),
        Some((Some(target),)) => Ok(target),
        Some((None,)) => Ok(cat_id),
    }
}

/// Has this user actually met the cat? Naming and reviewing are gated on it: you can only
/// name or judge a cat you've photographed yourself, not one you merely saw on the map.
pub async fn has_met(state: &AppState, cat_id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
    let row: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM sightings WHERE cat_id = $1 AND user_id = $2)",
    )
    .bind(cat_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    Ok(row.0)
}

/// Full detail for one cat: ranked names + all sightings. `viewer` is used to
/// flag which names the caller already liked. A merged cat redirects to the cat it
/// became, so old links keep working instead of 404ing.
pub async fn detail(state: &AppState, cat_id: Uuid, viewer: Option<Uuid>) -> Result<CatDetail, AppError> {
    let cat_id = resolve_redirect(state, cat_id).await?;

    let cat = sqlx::query_as::<_, Cat>(
        "SELECT id, created_by, display_name, created_at FROM cats WHERE id = $1",
    )
    .bind(cat_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    // Hidden names are dropped from the ranking; a hidden name can't win the label.
    let names = sqlx::query_as::<_, NameView>(
        "SELECT n.id, n.user_id, u.nickname, n.name,
                count(l.user_id) AS likes,
                coalesce(bool_or(l.user_id = $2), false) AS liked_by_me
         FROM cat_names n
         JOIN users u ON u.id = n.user_id
         LEFT JOIN name_likes l ON l.name_id = n.id
         WHERE n.cat_id = $1 AND NOT n.hidden
         GROUP BY n.id, n.user_id, u.nickname, n.name
         ORDER BY likes DESC, n.created_at ASC",
    )
    .bind(cat_id)
    .bind(viewer)
    .fetch_all(&state.db)
    .await?;

    let sightings = sqlx::query_as::<_, crate::models::cat::Sighting>(
        "SELECT id, user_id, cat_id, photo_url, thumb_url, lat, lng, taken_at, created_at
         FROM sightings WHERE cat_id = $1 ORDER BY created_at DESC",
    )
    .bind(cat_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(SightingView::from)
    .collect();

    let reviews = sqlx::query_as::<_, ReviewView>(
        "SELECT r.id, r.user_id, u.nickname, r.body, r.rating, r.created_at,
                count(l.user_id) AS likes,
                coalesce(bool_or(l.user_id = $2), false) AS liked_by_me
         FROM cat_reviews r
         JOIN users u ON u.id = r.user_id
         LEFT JOIN review_likes l ON l.review_id = r.id
         WHERE r.cat_id = $1 AND NOT r.hidden
         GROUP BY r.id, r.user_id, u.nickname, r.body, r.rating, r.created_at
         ORDER BY likes DESC, r.created_at DESC",
    )
    .bind(cat_id)
    .bind(viewer)
    .fetch_all(&state.db)
    .await?;

    // Whether the viewer has met this cat — drives whether the app shows the naming and
    // review composers at all. Anonymous viewers never can.
    let can_contribute = match viewer {
        Some(uid) => has_met(state, cat_id, uid).await?,
        None => false,
    };

    Ok(CatDetail {
        id: cat.id,
        display_name: cat.display_name,
        created_at: cat.created_at,
        names,
        reviews,
        sightings,
        can_contribute,
    })
}

/// Upsert this user's review of a cat, then return nothing (caller refetches detail).
pub async fn set_review(
    state: &AppState,
    cat_id: Uuid,
    user_id: Uuid,
    body: &str,
    rating: i16,
) -> Result<(), AppError> {
    let owner: Option<(Option<Uuid>,)> = sqlx::query_as("SELECT created_by FROM cats WHERE id = $1")
        .bind(cat_id)
        .fetch_optional(&state.db)
        .await?;
    let Some((owner,)) = owner else {
        return Err(AppError::NotFound);
    };

    // You can only judge a cat you've actually met.
    if !has_met(state, cat_id, user_id).await? {
        return Err(AppError::forbidden("Meet the cat before reviewing it."));
    }

    // Editing your own review clears a previous hide: you've fixed it.
    sqlx::query(
        "INSERT INTO cat_reviews (cat_id, user_id, body, rating) VALUES ($1, $2, $3, $4)
         ON CONFLICT (cat_id, user_id) DO UPDATE
             SET body = EXCLUDED.body, rating = EXCLUDED.rating, hidden = false",
    )
    .bind(cat_id)
    .bind(user_id)
    .bind(body)
    .bind(rating)
    .execute(&state.db)
    .await?;

    if let Some(owner) = owner {
        crate::services::notification::notify(state, owner, user_id, "review_added", Some(cat_id)).await;
    }
    Ok(())
}

/// Toggle a like on a review. Returns whether the like is now set. A user may like many
/// different reviews — only the same review twice toggles off.
pub async fn toggle_review_like(
    state: &AppState,
    review_id: Uuid,
    user_id: Uuid,
) -> Result<bool, AppError> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM cat_reviews WHERE id = $1")
        .bind(review_id)
        .fetch_optional(&state.db)
        .await?;
    if exists.is_none() {
        return Err(AppError::NotFound);
    }

    let deleted = sqlx::query("DELETE FROM review_likes WHERE review_id = $1 AND user_id = $2")
        .bind(review_id)
        .bind(user_id)
        .execute(&state.db)
        .await?;

    if deleted.rows_affected() > 0 {
        return Ok(false);
    }
    sqlx::query("INSERT INTO review_likes (review_id, user_id) VALUES ($1, $2)")
        .bind(review_id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(true)
}

/// Set (or replace) this user's name for a cat, then refresh the display name.
pub async fn set_name(state: &AppState, cat_id: Uuid, user_id: Uuid, name: &str) -> Result<(), AppError> {
    let owner: Option<(Option<Uuid>,)> = sqlx::query_as("SELECT created_by FROM cats WHERE id = $1")
        .bind(cat_id)
        .fetch_optional(&state.db)
        .await?;
    let Some((owner,)) = owner else {
        return Err(AppError::NotFound);
    };

    // You can only name a cat you've actually met.
    if !has_met(state, cat_id, user_id).await? {
        return Err(AppError::forbidden("Meet the cat before naming it."));
    }

    // `xmax = 0` is true only on a fresh INSERT, not the conflict UPDATE — so a re-name
    // edits in place (and un-hides, since the user just rewrote it) without pinging the
    // cat's owner a second time.
    let (name_id, fresh): (Uuid, bool) = sqlx::query_as(
        "INSERT INTO cat_names (cat_id, user_id, name) VALUES ($1, $2, $3)
         ON CONFLICT (cat_id, user_id) DO UPDATE SET name = EXCLUDED.name, hidden = false
         RETURNING id, (xmax = 0) AS fresh",
    )
    .bind(cat_id)
    .bind(user_id)
    .bind(name)
    .fetch_one(&state.db)
    .await?;

    // Naming a cat *is* agreeing with it: the user's single like moves to the name
    // they just gave, so a fresh name starts with one like.
    move_like(state, cat_id, user_id, name_id).await?;

    refresh_display_name(state, cat_id).await?;

    if fresh && let Some(owner) = owner {
        crate::services::notification::notify(state, owner, user_id, "name_added", Some(cat_id)).await;
    }
    Ok(())
}

/// Back a name. Tapping the name you already back removes your like; tapping any other
/// name moves your like to it — a user backs at most one name per cat.
pub async fn toggle_like(state: &AppState, name_id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT cat_id, user_id FROM cat_names WHERE id = $1")
            .bind(name_id)
            .fetch_optional(&state.db)
            .await?;
    let Some((cat_id, author_id)) = row else {
        return Err(AppError::NotFound);
    };

    let already = sqlx::query_as::<_, (Uuid,)>(
        "SELECT name_id FROM name_likes WHERE name_id = $1 AND user_id = $2",
    )
    .bind(name_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .is_some();

    if already {
        sqlx::query("DELETE FROM name_likes WHERE name_id = $1 AND user_id = $2")
            .bind(name_id)
            .bind(user_id)
            .execute(&state.db)
            .await?;
        refresh_display_name(state, cat_id).await?;
        return Ok(false);
    }

    move_like(state, cat_id, user_id, name_id).await?;
    refresh_display_name(state, cat_id).await?;

    // Tell the name's author their name just picked up a backer.
    crate::services::notification::notify(state, author_id, user_id, "name_liked", Some(cat_id)).await;
    Ok(true)
}

/// Points this user's single like for [cat_id] at [name_id], clearing any previous one.
async fn move_like(
    state: &AppState,
    cat_id: Uuid,
    user_id: Uuid,
    name_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM name_likes WHERE user_id = $1 AND cat_id = $2")
        .bind(user_id)
        .bind(cat_id)
        .execute(&state.db)
        .await?;
    sqlx::query("INSERT INTO name_likes (name_id, user_id, cat_id) VALUES ($1, $2, $3)")
        .bind(name_id)
        .bind(user_id)
        .bind(cat_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// Recompute `cats.display_name` = the most-liked visible name (ties: earliest). Hidden
/// names are ignored, so a hidden name can't hold the label.
pub async fn refresh_display_name(state: &AppState, cat_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE cats SET display_name = (
             SELECT n.name
             FROM cat_names n
             LEFT JOIN name_likes l ON l.name_id = n.id
             WHERE n.cat_id = $1 AND NOT n.hidden
             GROUP BY n.id, n.name, n.created_at
             ORDER BY count(l.user_id) DESC, n.created_at ASC
             LIMIT 1
         ) WHERE id = $1",
    )
    .bind(cat_id)
    .execute(&state.db)
    .await?;
    Ok(())
}
