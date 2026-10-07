use axum::{Json, extract::{Query, State}};
use serde_json::Value;

use crate::{
    error::AppError,
    handlers::sightings::PageQuery,
    middleware::auth::AuthUser,
    response, services,
    state::AppState,
};

/// `GET /notifications?limit=&cursor=` — your inbox, newest first, paged.
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<PageQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = crate::paging::limit(q.limit);
    let cursor = crate::paging::decode(q.cursor.as_deref())?;
    let page = services::notification::list(&state, auth.user.id, limit, cursor).await?;
    Ok(response::ok(page))
}

/// `GET /notifications/unread-count` — the little badge on the bell.
pub async fn unread(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, AppError> {
    let count = services::notification::unread_count(&state, auth.user.id).await?;
    Ok(response::ok(serde_json::json!({ "count": count })))
}

/// `POST /notifications/seen` — mark everything read (called when the inbox opens).
pub async fn seen(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, AppError> {
    services::notification::mark_all_seen(&state, auth.user.id).await?;
    Ok(response::message("All caught up."))
}
