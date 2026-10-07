use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{error::AppError, middleware::auth::{AuthUser, VerifiedUser}, response, services, state::AppState, validation};

#[derive(Deserialize)]
pub struct BboxQuery {
    pub min_lat: f64,
    pub min_lng: f64,
    pub max_lat: f64,
    pub max_lng: f64,
}

/// `GET /cats?min_lat&min_lng&max_lat&max_lng` — map markers in the viewport.
pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<BboxQuery>,
) -> Result<Json<Value>, AppError> {
    if q.min_lat > q.max_lat || q.min_lng > q.max_lng {
        return Err(AppError::bad_request("That map area looks backwards."));
    }
    let markers = services::cat::in_bbox(&state, q.min_lat, q.min_lng, q.max_lat, q.max_lng).await?;
    Ok(response::ok(markers))
}

/// `GET /cats/{id}` — names (ranked by likes) and every sighting.
pub async fn detail(
    State(state): State<AppState>,
    viewer: Option<AuthUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let viewer_id = viewer.map(|a| a.user.id);
    let cat = services::cat::detail(&state, id, viewer_id).await?;
    Ok(response::ok(cat))
}

#[derive(Deserialize)]
pub struct NameRequest {
    pub name: String,
}

/// `POST /cats/{id}/names` — set your name for this cat.
pub async fn set_name(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<NameRequest>,
) -> Result<Json<Value>, AppError> {
    let name = req.name.trim();
    if !validation::cat_name(name) {
        return Err(AppError::bad_request("Cat names are 1-40 characters."));
    }
    state.check_write(auth.user.id)?;
    services::cat::set_name(&state, id, auth.user.id, name).await?;
    let cat = services::cat::detail(&state, id, Some(auth.user.id)).await?;
    Ok(response::ok(cat))
}

/// `POST /names/{id}/like` — toggle a like ("I agree with this name").
pub async fn like(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.check_write(auth.user.id)?;
    let liked = services::cat::toggle_like(&state, id, auth.user.id).await?;
    Ok(response::ok(serde_json::json!({ "liked": liked })))
}

#[derive(Deserialize)]
pub struct ReviewRequest {
    pub body: String,
    pub rating: i16,
}

/// `POST /cats/{id}/reviews` — write (or replace) your review: a note and a 0-10 rating.
pub async fn set_review(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ReviewRequest>,
) -> Result<Json<Value>, AppError> {
    let body = req.body.trim();
    if !validation::cat_review(body) {
        return Err(AppError::bad_request("Reviews are 1-500 characters."));
    }
    if !(0..=10).contains(&req.rating) {
        return Err(AppError::bad_request("Ratings go from 0 to 10."));
    }
    state.check_write(auth.user.id)?;
    services::cat::set_review(&state, id, auth.user.id, body, req.rating).await?;
    let cat = services::cat::detail(&state, id, Some(auth.user.id)).await?;
    Ok(response::ok(cat))
}

/// `POST /reviews/{id}/like` — toggle a like on someone's review (many allowed).
pub async fn like_review(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    state.check_write(auth.user.id)?;
    let liked = services::cat::toggle_review_like(&state, id, auth.user.id).await?;
    Ok(response::ok(serde_json::json!({ "liked": liked })))
}
