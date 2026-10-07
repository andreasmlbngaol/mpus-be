use serde::Serialize;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

/// One notification as the inbox shows it: who did it, what they did, and which cat.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct NotificationView {
    pub id: Uuid,
    pub kind: String,
    pub actor_nickname: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub cat_id: Option<Uuid>,
    pub cat_name: Option<String>,
    pub seen: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Record a notification for [user_id]. Best-effort by design: a notification is a
/// nicety, so a failure here must never sink the action that triggered it. Never notify
/// someone about their own action.
pub async fn notify(
    state: &AppState,
    user_id: Uuid,
    actor_id: Uuid,
    kind: &str,
    cat_id: Option<Uuid>,
) {
    if user_id == actor_id {
        return;
    }
    let _ = sqlx::query(
        "INSERT INTO notifications (user_id, actor_id, kind, cat_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(actor_id)
    .bind(kind)
    .bind(cat_id)
    .execute(&state.db)
    .await;

    // Mirror it to the device if push is configured. Spawned, not awaited: FCM is a
    // network round-trip and must never slow down (or sink) the action that fired this.
    if state.push.is_some() {
        let state = state.clone();
        let kind = kind.to_string();
        tokio::spawn(async move { push(&state, user_id, actor_id, &kind, cat_id).await });
    }
}

/// Fan a freshly-inserted notification out to every device the user has registered.
/// Silent when push isn't configured; a dead token is dropped from the table.
async fn push(state: &AppState, user_id: Uuid, actor_id: Uuid, kind: &str, cat_id: Option<Uuid>) {
    let Some(push) = state.push.as_ref() else {
        return;
    };

    let actor: Option<(String,)> = sqlx::query_as("SELECT nickname FROM users WHERE id = $1")
        .bind(actor_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let actor = actor.map(|(n,)| n).unwrap_or_else(|| "Someone".to_string());

    let cat: Option<(Option<String>,)> = match cat_id {
        Some(id) => sqlx::query_as("SELECT display_name FROM cats WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let cat = cat
        .and_then(|(n,)| n)
        .unwrap_or_else(|| "this cat".to_string());

    // A distinct title per kind so the shade reads at a glance, not a wall of "MPUS".
    let (title, body) = match kind {
        "name_liked" => (
            "Name love",
            format!("{actor} liked your name for {cat}"),
        ),
        "name_added" => (
            "New name",
            format!("{actor} named a cat you found: {cat}"),
        ),
        "review_added" => (
            "New review",
            format!("{actor} reviewed a cat you found: {cat}"),
        ),
        "merge_requested" => (
            "Merge request",
            format!("{actor} wants to merge a cat you own: {cat}"),
        ),
        "merge_resolved" => (
            "Merge update",
            format!("{actor} resolved a merge request for {cat}"),
        ),
        _ => ("MPUS", format!("{actor} did something with {cat}")),
    };

    let tokens: Vec<(String,)> = match sqlx::query_as("SELECT token FROM device_tokens WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "failed to load device tokens for push");
            return;
        }
    };

    for (token,) in tokens {
        let data = serde_json::json!({
            "kind": kind,
            "cat_id": cat_id.map(|c| c.to_string()),
        });
        if let Err(crate::services::push::PushError::Dead) =
            push.send(&state.http, &token, title, &body, data).await
        {
            let _ = sqlx::query("DELETE FROM device_tokens WHERE token = $1")
                .bind(&token)
                .execute(&state.db)
                .await;
        }
    }
}

/// Claim [token] for [user_id] (insert or move it off whoever had it before).
pub async fn register_device(
    state: &AppState,
    user_id: Uuid,
    token: &str,
    platform: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO device_tokens (token, user_id, platform, updated_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (token) DO UPDATE
             SET user_id = EXCLUDED.user_id, platform = EXCLUDED.platform, updated_at = now()",
    )
    .bind(token)
    .bind(user_id)
    .bind(platform)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Forget a device token (called on logout so a signed-out phone stops getting pushes).
pub async fn unregister_device(state: &AppState, token: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM device_tokens WHERE token = $1")
        .bind(token)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// The caller's inbox, newest first, keyset-paged.
pub async fn list(
    state: &AppState,
    user_id: Uuid,
    limit: i64,
    cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)>,
) -> Result<crate::models::cat::Page<NotificationView>, AppError> {
    let (after_ts, after_id) = match cursor {
        Some((ts, id)) => (Some(ts), Some(id)),
        None => (None, None),
    };

    let mut rows = sqlx::query_as::<_, NotificationView>(
        "SELECT n.id, n.kind,
                a.nickname AS actor_nickname, a.avatar_url AS actor_avatar_url,
                n.cat_id, c.display_name AS cat_name,
                (n.seen_at IS NOT NULL) AS seen, n.created_at
         FROM notifications n
         LEFT JOIN users a ON a.id = n.actor_id
         LEFT JOIN cats  c ON c.id = n.cat_id
         WHERE n.user_id = $1
           AND ($2::timestamptz IS NULL OR (n.created_at, n.id) < ($2, $3))
         ORDER BY n.created_at DESC, n.id DESC
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
        rows.last().map(|n| crate::paging::encode(n.created_at, n.id))
    } else {
        None
    };

    Ok(crate::models::cat::Page { items: rows, next_cursor })
}

/// Unread count, for the little badge on the bell.
pub async fn unread_count(state: &AppState, user_id: Uuid) -> Result<i64, AppError> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND seen_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    Ok(count)
}

/// Mark everything as seen — called when the inbox opens.
pub async fn mark_all_seen(state: &AppState, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE notifications SET seen_at = now() WHERE user_id = $1 AND seen_at IS NULL")
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(())
}
