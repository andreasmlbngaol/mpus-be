use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{error::AppError, middleware::auth::{AuthUser, VerifiedUser}, response, services, state::AppState};

#[derive(Deserialize)]
pub struct MergeRequest {
    /// The cat that should win (keep its id and links).
    pub target_cat_id: Uuid,
}

/// `POST /cats/{id}/merge` — ask to fold this cat into another. If you own both, it
/// happens now; otherwise the other owner has to agree.
pub async fn request(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<MergeRequest>,
) -> Result<Json<Value>, AppError> {
    state.check_write(auth.user.id)?;
    let view = services::merge::request(&state, auth.user.id, id, req.target_cat_id).await?;
    Ok(response::ok(view))
}

/// `GET /merges/pending` — merges waiting on your yes/no.
pub async fn pending(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Value>, AppError> {
    let views = services::merge::pending_for(&state, auth.user.id).await?;
    Ok(response::ok(views))
}

/// `POST /merges/{id}/approve` — the other owner agrees; the merge completes.
pub async fn approve(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.check_write(auth.user.id)?;
    let view = services::merge::approve(&state, auth.user.id, id).await?;
    Ok(response::ok(view))
}

/// `POST /merges/{id}/reject` — the other owner says no.
pub async fn reject(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.check_write(auth.user.id)?;
    services::merge::reject(&state, auth.user.id, id).await?;
    Ok(response::message("Merge declined."))
}

/// `POST /merges/{id}/undo` — put a completed merge back the way it was.
pub async fn undo(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.check_write(auth.user.id)?;
    services::merge::undo(&state, auth.user.id, id).await?;
    Ok(response::message("Merge undone."))
}
