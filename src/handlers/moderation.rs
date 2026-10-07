use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    error::AppError,
    middleware::auth::VerifiedUser,
    response,
    services::{self, moderation::TargetKind},
    state::AppState,
    validation,
};

#[derive(Deserialize)]
pub struct ReportRequest {
    /// "name" or "review".
    pub target_kind: String,
    pub target_id: Uuid,
    pub reason: Option<String>,
}

/// `POST /reports` — flag a name or review. Idempotent per user; the item hides itself
/// once enough distinct people agree.
pub async fn create(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Json(req): Json<ReportRequest>,
) -> Result<Json<Value>, AppError> {
    let kind = match req.target_kind.as_str() {
        "name" => TargetKind::Name,
        "review" => TargetKind::Review,
        _ => return Err(AppError::bad_request("You can only report a name or a review.")),
    };
    let reason = req.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
    if let Some(reason) = reason
        && !validation::report_reason(reason)
    {
        return Err(AppError::bad_request("That report note is too long."));
    }

    state.check_write(auth.user.id)?;
    let hidden = services::moderation::report(&state, auth.user.id, kind, req.target_id, reason).await?;
    Ok(response::ok(serde_json::json!({ "hidden": hidden })))
}
