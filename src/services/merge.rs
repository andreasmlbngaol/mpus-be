use serde::Serialize;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

/// A merge request as the client sees it: which two cats, who asked, and whether it's
/// waiting on the other owner.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MergeView {
    pub id: Uuid,
    pub source_cat_id: Uuid,
    pub target_cat_id: Uuid,
    pub source_name: Option<String>,
    pub target_name: Option<String>,
    pub requested_by: Uuid,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// The cat's owner, as far as merge cares: whoever created it. `None` (owner deleted
/// their account) means nobody can approve, so such a cat can only be merged by someone
/// who owns both sides.
async fn owner_of(state: &AppState, cat_id: Uuid) -> Result<Option<Uuid>, AppError> {
    let row: Option<(Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as("SELECT created_by, merged_into FROM cats WHERE id = $1")
            .bind(cat_id)
            .fetch_optional(&state.db)
            .await?;
    match row {
        None => Err(AppError::NotFound),
        Some((_, Some(_))) => Err(AppError::conflict("That cat was already merged into another.")),
        Some((owner, None)) => Ok(owner),
    }
}

/// Ask to merge [source] into [target] (the target survives). If the requester owns both
/// cats, it happens at once. Otherwise the other owner has to agree first — nobody can
/// dissolve a cat someone else built.
pub async fn request(
    state: &AppState,
    requester: Uuid,
    source: Uuid,
    target: Uuid,
) -> Result<MergeView, AppError> {
    if source == target {
        return Err(AppError::bad_request("A cat can't be merged with itself."));
    }
    let source_owner = owner_of(state, source).await?;
    let target_owner = owner_of(state, target).await?;

    let owns_source = source_owner == Some(requester);
    let owns_target = target_owner == Some(requester);
    if !owns_source && !owns_target {
        return Err(AppError::forbidden(
            "You can only merge cats you've created.",
        ));
    }

    // A pending request between the same pair shouldn't stack.
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM merge_requests
         WHERE status = 'pending' AND source_cat_id = $1 AND target_cat_id = $2",
    )
    .bind(source)
    .bind(target)
    .fetch_optional(&state.db)
    .await?;
    if existing.is_some() {
        return Err(AppError::conflict("There's already a pending merge for these cats."));
    }

    let both = owns_source && owns_target;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO merge_requests
            (source_cat_id, target_cat_id, requested_by, source_approved, target_approved, status)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(source)
    .bind(target)
    .bind(requester)
    .bind(owns_source)
    .bind(owns_target)
    .bind(if both { "merged" } else { "pending" })
    .fetch_one(&state.db)
    .await?;

    if both {
        perform(state, id, source, target).await?;
    } else {
        // Tell the other owner there's a merge waiting on them.
        let other = if owns_source { target_owner } else { source_owner };
        if let Some(other) = other {
            crate::services::notification::notify(
                state,
                other,
                requester,
                "merge_requested",
                Some(target),
            )
            .await;
        }
    }

    view(state, id).await
}

/// Approve a pending merge. The other owner's "yes" completes it.
pub async fn approve(state: &AppState, requester: Uuid, merge_id: Uuid) -> Result<MergeView, AppError> {
    let row: Option<(Uuid, Uuid, Uuid, String, bool, bool)> = sqlx::query_as(
        "SELECT source_cat_id, target_cat_id, requested_by, status, source_approved, target_approved
         FROM merge_requests WHERE id = $1",
    )
    .bind(merge_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((source, target, _requested_by, status, mut s_ok, mut t_ok)) = row else {
        return Err(AppError::NotFound);
    };
    if status != "pending" {
        return Err(AppError::conflict("That merge isn't pending anymore."));
    }

    let source_owner = owner_of(state, source).await?;
    let target_owner = owner_of(state, target).await?;
    match requester {
        u if source_owner == Some(u) => s_ok = true,
        u if target_owner == Some(u) => t_ok = true,
        _ => return Err(AppError::forbidden("You can only approve merges for cats you've created.")),
    }

    let done = s_ok && t_ok;
    sqlx::query(
        "UPDATE merge_requests SET source_approved = $2, target_approved = $3, status = $4
         WHERE id = $1",
    )
    .bind(merge_id)
    .bind(s_ok)
    .bind(t_ok)
    .bind(if done { "merged" } else { "pending" })
    .execute(&state.db)
    .await?;

    if done {
        perform(state, merge_id, source, target).await?;
    }
    view(state, merge_id).await
}

/// Reject a pending merge — the other owner says no.
pub async fn reject(state: &AppState, requester: Uuid, merge_id: Uuid) -> Result<(), AppError> {
    let row: Option<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT source_cat_id, target_cat_id, requested_by, status FROM merge_requests WHERE id = $1",
    )
    .bind(merge_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((source, target, requested_by, status)) = row else {
        return Err(AppError::NotFound);
    };
    if status != "pending" {
        return Err(AppError::conflict("That merge isn't pending anymore."));
    }

    let source_owner = owner_of(state, source).await?;
    let target_owner = owner_of(state, target).await?;
    let is_owner = source_owner == Some(requester) || target_owner == Some(requester);
    if !is_owner {
        return Err(AppError::forbidden("You can only reject merges for cats you've created."));
    }

    sqlx::query("UPDATE merge_requests SET status = 'rejected', resolved_at = now() WHERE id = $1")
        .bind(merge_id)
        .execute(&state.db)
        .await?;

    // Let the requester know it was turned down.
    crate::services::notification::notify(state, requested_by, requester, "merge_resolved", Some(target))
        .await;
    Ok(())
}

/// Do the actual merge: re-point everything from source to target, remember what moved
/// so it can be undone, and redirect the old id.
async fn perform(state: &AppState, merge_id: Uuid, source: Uuid, target: Uuid) -> Result<(), AppError> {
    let mut tx = state.db.begin().await?;

    // Sightings: no uniqueness, they all just move.
    sqlx::query(
        "INSERT INTO merge_moves (merge_id, kind, moved_id)
         SELECT $1, 'sighting', id FROM sightings WHERE cat_id = $2",
    )
    .bind(merge_id)
    .bind(source)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE sightings SET cat_id = $2 WHERE cat_id = $1")
        .bind(source)
        .bind(target)
        .execute(&mut *tx)
        .await?;

    // Names: one per (user, cat). If both cats hold a name from the same user, the
    // target's wins and the duplicate is dropped — a duplicate cat merge rarely hits
    // this, and it can't be undone for the dropped row.
    sqlx::query(
        "INSERT INTO merge_moves (merge_id, kind, moved_id)
         SELECT $1, 'name', n.id FROM cat_names n
         WHERE n.cat_id = $2
           AND NOT EXISTS (SELECT 1 FROM cat_names t WHERE t.cat_id = $3 AND t.user_id = n.user_id)",
    )
    .bind(merge_id)
    .bind(source)
    .bind(target)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM cat_names WHERE cat_id = $1 AND user_id IN (SELECT user_id FROM cat_names WHERE cat_id = $2)")
        .bind(source)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE cat_names SET cat_id = $2 WHERE cat_id = $1")
        .bind(source)
        .bind(target)
        .execute(&mut *tx)
        .await?;

    // Name likes: follow their name's new cat, but the one-like-per-cat rule means a
    // user already backing a target name keeps that one.
    sqlx::query(
        "DELETE FROM name_likes WHERE cat_id = $1
           AND user_id IN (SELECT user_id FROM name_likes WHERE cat_id = $2)",
    )
    .bind(source)
    .bind(target)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE name_likes SET cat_id = $2 WHERE cat_id = $1")
        .bind(source)
        .bind(target)
        .execute(&mut *tx)
        .await?;

    // Reviews: one per (user, cat); target's wins on conflict.
    sqlx::query(
        "INSERT INTO merge_moves (merge_id, kind, moved_id)
         SELECT $1, 'review', r.id FROM cat_reviews r
         WHERE r.cat_id = $2
           AND NOT EXISTS (SELECT 1 FROM cat_reviews t WHERE t.cat_id = $3 AND t.user_id = r.user_id)",
    )
    .bind(merge_id)
    .bind(source)
    .bind(target)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM cat_reviews WHERE cat_id = $1 AND user_id IN (SELECT user_id FROM cat_reviews WHERE cat_id = $2)")
        .bind(source)
        .bind(target)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE cat_reviews SET cat_id = $2 WHERE cat_id = $1")
        .bind(source)
        .bind(target)
        .execute(&mut *tx)
        .await?;

    // The source cat becomes an alias: old links resolve to the target.
    sqlx::query("UPDATE cats SET merged_into = $2 WHERE id = $1")
        .bind(source)
        .bind(target)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE merge_requests SET status = 'merged', resolved_at = now() WHERE id = $1")
        .bind(merge_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    crate::services::cat::refresh_display_name(state, target).await?;
    Ok(())
}

/// Undo a merge within its window: put every moved row back and un-redirect the source.
pub async fn undo(state: &AppState, requester: Uuid, merge_id: Uuid) -> Result<(), AppError> {
    let row: Option<(Uuid, Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT source_cat_id, target_cat_id, requested_by, status FROM merge_requests WHERE id = $1",
    )
    .bind(merge_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((source, target, requested_by, status)) = row else {
        return Err(AppError::NotFound);
    };
    if status != "merged" {
        return Err(AppError::conflict("Only a completed merge can be undone."));
    }

    // The requester can always undo; the source cat's creator can too.
    let source_owner: Option<(Option<Uuid>,)> =
        sqlx::query_as("SELECT created_by FROM cats WHERE id = $1")
            .bind(source)
            .fetch_optional(&state.db)
            .await?;
    if requester != requested_by && source_owner.and_then(|o| o.0) != Some(requester) {
        return Err(AppError::forbidden("You can only undo your own merges."));
    }

    let mut tx = state.db.begin().await?;

    sqlx::query("UPDATE sightings SET cat_id = $2 WHERE id IN (SELECT moved_id FROM merge_moves WHERE merge_id = $1 AND kind = 'sighting')")
        .bind(merge_id).bind(source).execute(&mut *tx).await?;
    sqlx::query("UPDATE cat_names SET cat_id = $2 WHERE id IN (SELECT moved_id FROM merge_moves WHERE merge_id = $1 AND kind = 'name')")
        .bind(merge_id).bind(source).execute(&mut *tx).await?;
    sqlx::query("UPDATE cat_reviews SET cat_id = $2 WHERE id IN (SELECT moved_id FROM merge_moves WHERE merge_id = $1 AND kind = 'review')")
        .bind(merge_id).bind(source).execute(&mut *tx).await?;
    sqlx::query("UPDATE name_likes SET cat_id = $2 WHERE name_id IN (SELECT moved_id FROM merge_moves WHERE merge_id = $1 AND kind = 'name')")
        .bind(merge_id).bind(source).execute(&mut *tx).await?;

    sqlx::query("UPDATE cats SET merged_into = NULL WHERE id = $1")
        .bind(source)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE merge_requests SET status = 'undone', resolved_at = now() WHERE id = $1")
        .bind(merge_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    crate::services::cat::refresh_display_name(state, source).await?;
    crate::services::cat::refresh_display_name(state, target).await?;
    Ok(())
}

/// Pending merges waiting on the caller's approval (they own a side but didn't ask).
pub async fn pending_for(state: &AppState, user: Uuid) -> Result<Vec<MergeView>, AppError> {
    let rows = sqlx::query_as::<_, MergeView>(
        "SELECT m.id, m.source_cat_id, m.target_cat_id,
                sc.display_name AS source_name, tc.display_name AS target_name,
                m.requested_by, m.status, m.created_at
         FROM merge_requests m
         JOIN cats sc ON sc.id = m.source_cat_id
         JOIN cats tc ON tc.id = m.target_cat_id
         WHERE m.status = 'pending' AND m.requested_by <> $1
           AND (sc.created_by = $1 OR tc.created_by = $1)",
    )
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    Ok(rows)
}

async fn view(state: &AppState, merge_id: Uuid) -> Result<MergeView, AppError> {
    sqlx::query_as::<_, MergeView>(
        "SELECT m.id, m.source_cat_id, m.target_cat_id,
                sc.display_name AS source_name, tc.display_name AS target_name,
                m.requested_by, m.status, m.created_at
         FROM merge_requests m
         JOIN cats sc ON sc.id = m.source_cat_id
         JOIN cats tc ON tc.id = m.target_cat_id
         WHERE m.id = $1",
    )
    .bind(merge_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)
}
