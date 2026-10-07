use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    error::AppError,
    middleware::auth::{AuthUser, VerifiedUser},
    models::cat::{SightingCreated, SightingView},
    response, services,
    state::AppState,
    validation,
};

/// `POST /sightings` — multipart: `photo`, `lat`, `lng`, optional `taken_at`.
///
/// Stores the photo as a pending sighting, then hands back the most similar
/// existing cats. Nothing is linked automatically — the user decides.
pub async fn create(
    State(state): State<AppState>,
    auth: VerifiedUser,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let r = &state.config.rate;
    if !state
        .rate
        .check(&crate::ratelimit::key_str(&auth.user.id.to_string(), "sighting"), r.sighting_max, r.sighting_window)
    {
        return Err(AppError::too_many(
            "That's a lot of cats at once. Give it a minute and try again.",
        ));
    }

    let max_bytes = state.config.sighting.max_bytes;
    let mut photo: Option<Vec<u8>> = None;
    let mut lat: Option<f64> = None;
    let mut lng: Option<f64> = None;
    let mut taken_at: Option<chrono::DateTime<chrono::Utc>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("Invalid upload form: {e}")))?
    {
        match field.name() {
            Some("photo") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| AppError::bad_request("We couldn't read that upload."))?;
                if bytes.is_empty() {
                    return Err(AppError::bad_request("That file is empty."));
                }
                if bytes.len() > max_bytes {
                    let mb = max_bytes / (1024 * 1024);
                    return Err(AppError::bad_request(format!(
                        "Cat photos have to be under {mb} MB."
                    )));
                }
                photo = Some(bytes.to_vec());
            }
            Some("lat") => lat = Some(parse_coord(&field.text().await, "lat")?),
            Some("lng") => lng = Some(parse_coord(&field.text().await, "lng")?),
            Some("taken_at") => {
                if let Ok(t) = field.text().await {
                    taken_at = chrono::DateTime::parse_from_rfc3339(t.trim())
                        .ok()
                        .map(|d| d.with_timezone(&chrono::Utc));
                }
            }
            _ => {}
        }
    }

    let photo = photo.ok_or_else(|| {
        AppError::bad_request("No photo found. Send it as the 'photo' field.")
    })?;
    let lat = lat.ok_or_else(|| AppError::bad_request("Where was this cat? Send 'lat'."))?;
    let lng = lng.ok_or_else(|| AppError::bad_request("Where was this cat? Send 'lng'."))?;

    // Everything below is CPU-bound: decode, re-encode, embed.
    let sighting_cfg = state.config.sighting.clone();
    let embed = state.embed.clone();
    let photo_for_work = photo.clone();
    let (webp, thumb, embedding) = tokio::task::spawn_blocking(move || {
        // The stored photo is center-cropped to 1:1 so what the app shows (Snap preview,
        // cat hero, map marker) is exactly the frame that was saved. The embedding still
        // reads the original full frame — the model wants the whole cat, not a crop.
        let webp = services::image::square_webp(
            &photo_for_work,
            sighting_cfg.max_dimension,
            sighting_cfg.webp_quality,
        )?;
        let thumb = services::image::thumbnail(
            &photo_for_work,
            sighting_cfg.thumb_max_dim,
            sighting_cfg.webp_quality,
        )?;
        let embedding = embed.embed(&photo_for_work)?;
        Ok::<_, AppError>((webp, thumb, embedding))
    })
    .await
    .map_err(|e| AppError::internal(format!("photo task failed: {e}")))??;

    let dir = &state.config.upload_dir;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let photo_id = Uuid::new_v4();
    let photo_file = format!("cat-{photo_id}.webp");
    let thumb_file = format!("cat-{photo_id}-thumb.webp");
    tokio::fs::write(dir.join(&photo_file), &webp)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    tokio::fs::write(dir.join(&thumb_file), &thumb)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let base = &state.config.public_base_url;
    let photo_url = format!("{base}/avatars/{photo_file}");
    let thumb_url = format!("{base}/avatars/{thumb_file}");

    let sighting = services::sighting::insert(
        &state,
        auth.user.id,
        photo_url,
        thumb_url,
        &embedding,
        lat,
        lng,
        taken_at,
    )
    .await?;

    let candidates = services::sighting::candidates(
        &state,
        &embedding,
        state.config.embed.top_k,
        state.config.embed.min_similarity,
    )
    .await?;

    Ok(response::ok(SightingCreated {
        sighting: SightingView::from(sighting),
        candidates,
    }))
}

/// Shared paging query for list endpoints: `?limit=&cursor=`.
#[derive(Deserialize)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Deserialize)]
pub struct ResolveRequest {
    /// Link to an existing cat...
    pub cat_id: Option<Uuid>,
    /// ...or name a new one.
    pub name: Option<String>,
}

/// `POST /sightings/{id}/resolve` — the user's decision: link or create.
pub async fn resolve(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Path(id): Path<Uuid>,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<Value>, AppError> {
    // Ownership check first: you can only resolve your own sighting.
    services::sighting::by_id_owned(&state, id, auth.user.id).await?;

    match (req.cat_id, req.name.as_deref().map(str::trim)) {
        (Some(cat_id), None) => {
            services::sighting::link_to_cat(&state, id, cat_id).await?;
            Ok(response::ok(serde_json::json!({ "cat_id": cat_id })))
        }
        (None, Some(name)) if !name.is_empty() => {
            if !validation::cat_name(name) {
                return Err(AppError::bad_request("Cat names are 1-40 characters."));
            }
            let cat_id = services::sighting::create_cat_with_name(&state, auth.user.id, id, name).await?;
            Ok(response::ok(serde_json::json!({ "cat_id": cat_id })))
        }
        _ => Err(AppError::bad_request(
            "Send either a cat_id to link, or a name to start a new cat.",
        )),
    }
}

/// `GET /me/sightings?limit=&cursor=` — your own uploads, newest first, paged.
pub async fn mine(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<PageQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = crate::paging::limit(q.limit);
    let cursor = crate::paging::decode(q.cursor.as_deref())?;
    let page = services::sighting::list_for_user(&state, auth.user.id, limit, cursor).await?;
    Ok(response::ok(page))
}

fn parse_coord(text: &Result<String, axum::extract::multipart::MultipartError>, which: &str) -> Result<f64, AppError> {
    let raw = text
        .as_ref()
        .map_err(|_| AppError::bad_request(format!("We couldn't read '{which}'.")))?;
    let v: f64 = raw
        .trim()
        .parse()
        .map_err(|_| AppError::bad_request(format!("'{which}' needs to be a number.")))?;
    if !v.is_finite() {
        return Err(AppError::bad_request(format!("'{which}' needs to be a number.")));
    }
    Ok(v)
}
