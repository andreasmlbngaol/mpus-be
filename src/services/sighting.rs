use uuid::Uuid;

use crate::{
    error::AppError,
    models::cat::{Candidate, MySightings, Sighting, SightingView},
    state::AppState,
};

/// Insert a new pending sighting (cat_id = NULL until the user decides).
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    state: &AppState,
    user_id: Uuid,
    photo_url: String,
    thumb_url: String,
    embedding: &[f32],
    lat: f64,
    lng: f64,
    taken_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Sighting, AppError> {
    let vector = pgvector_literal(embedding);

    sqlx::query_as::<_, Sighting>(
        "INSERT INTO sightings
            (user_id, photo_url, thumb_url, embedding, lat, lng, taken_at)
         VALUES ($1, $2, $3, $4::vector, $5, $6, $7)
         RETURNING id, user_id, cat_id, photo_url, thumb_url, lat, lng, taken_at, created_at",
    )
    .bind(user_id)
    .bind(photo_url)
    .bind(thumb_url)
    .bind(vector)
    .bind(lat)
    .bind(lng)
    .bind(taken_at)
    .fetch_one(&state.db)
    .await
    .map_err(Into::into)
}

/// Find the most similar existing cats. One best sighting per cat, ranked by
/// cosine similarity. Anything below `min_similarity` is dropped — a weak match is
/// noise, and showing it would tell the user two unrelated cats are the same.
pub async fn candidates(
    state: &AppState,
    embedding: &[f32],
    top_k: u32,
    min_similarity: f32,
) -> Result<Vec<Candidate>, AppError> {
    let vector = pgvector_literal(embedding);

    let rows: Vec<(Uuid, String, Option<String>, f64)> = sqlx::query_as(
        "SELECT DISTINCT ON (s.cat_id)
                s.cat_id, s.thumb_url, c.display_name,
                1 - (s.embedding <=> $1::vector) AS similarity
         FROM sightings s
         JOIN cats c ON c.id = s.cat_id
         WHERE s.cat_id IS NOT NULL
           AND 1 - (s.embedding <=> $1::vector) >= $3
         ORDER BY s.cat_id, s.embedding <=> $1::vector
         LIMIT $2",
    )
    .bind(vector)
    .bind(top_k as i64)
    .bind(min_similarity as f64)
    .fetch_all(&state.db)
    .await?;

    // Re-sort the per-cat winners by similarity (DISTINCT ON forces cat_id order).
    let mut candidates: Vec<Candidate> = rows
        .into_iter()
        .map(|(cat_id, thumb_url, display_name, similarity)| Candidate {
            cat_id,
            thumb_url,
            display_name,
            similarity: similarity as f32,
        })
        .collect();
    candidates.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    Ok(candidates)
}

/// Point a pending sighting at an existing cat.
pub async fn link_to_cat(state: &AppState, sighting_id: Uuid, cat_id: Uuid) -> Result<(), AppError> {
    let res = sqlx::query("UPDATE sightings SET cat_id = $2 WHERE id = $1 AND cat_id IS NULL")
        .bind(sighting_id)
        .bind(cat_id)
        .execute(&state.db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::conflict(
            "This sighting was already sorted, or it isn't yours.",
        ));
    }
    Ok(())
}

/// Create a brand-new cat, attach the sighting, and give it the first name.
pub async fn create_cat_with_name(
    state: &AppState,
    user_id: Uuid,
    sighting_id: Uuid,
    name: &str,
) -> Result<Uuid, AppError> {
    let mut tx = state.db.begin().await?;

    let (cat_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO cats (created_by, display_name) VALUES ($1, $2) RETURNING id",
    )
    .bind(user_id)
    .bind(name)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO cat_names (cat_id, user_id, name) VALUES ($1, $2, $3)")
        .bind(cat_id)
        .bind(user_id)
        .bind(name)
        .execute(&mut *tx)
        .await?;

    let res = sqlx::query("UPDATE sightings SET cat_id = $2 WHERE id = $1 AND cat_id IS NULL")
        .bind(sighting_id)
        .bind(cat_id)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::conflict(
            "This sighting was already sorted, or it isn't yours.",
        ));
    }

    tx.commit().await?;
    Ok(cat_id)
}

/// Load one sighting, enforcing ownership. `NotFound` if missing or not theirs.
pub async fn by_id_owned(state: &AppState, sighting_id: Uuid, user_id: Uuid) -> Result<Sighting, AppError> {
    sqlx::query_as::<_, Sighting>(
        "SELECT id, user_id, cat_id, photo_url, thumb_url, lat, lng, taken_at, created_at
         FROM sightings WHERE id = $1 AND user_id = $2",
    )
    .bind(sighting_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}

/// A user's own sightings, newest first (pending ones included), keyset-paged,
/// with the totals the profile tab needs so it never has to count a partial page.
pub async fn list_for_user(
    state: &AppState,
    user_id: Uuid,
    limit: i64,
    cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)>,
) -> Result<MySightings, AppError> {
    let (after_ts, after_id) = match cursor {
        Some((ts, id)) => (Some(ts), Some(id)),
        None => (None, None),
    };

    let (sightings_count, cats_count, unnamed_count): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*),
                count(DISTINCT cat_id),
                count(*) FILTER (WHERE cat_id IS NULL)
         FROM sightings WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;

    // Fetch one extra row to learn whether another page exists.
    let mut rows = sqlx::query_as::<_, Sighting>(
        "SELECT id, user_id, cat_id, photo_url, thumb_url, lat, lng, taken_at, created_at
         FROM sightings
         WHERE user_id = $1
           AND ($2::timestamptz IS NULL OR (created_at, id) < ($2, $3))
         ORDER BY created_at DESC, id DESC
         LIMIT $4",
    )
    .bind(user_id)
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
        rows.last().map(|s| crate::paging::encode(s.created_at, s.id))
    } else {
        None
    };

    Ok(MySightings {
        items: rows.into_iter().map(SightingView::from).collect(),
        next_cursor,
        sightings_count,
        cats_count,
        unnamed_count,
    })
}

/// Render a float vector as a pgvector literal: `[1.0,2.0,...]`.
fn pgvector_literal(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 8 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&x.to_string());
    }
    s.push(']');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pgvector_literal_formats_correctly() {
        assert_eq!(pgvector_literal(&[1.0, 2.5, -3.0]), "[1,2.5,-3]");
        assert_eq!(pgvector_literal(&[]), "[]");
    }
}
