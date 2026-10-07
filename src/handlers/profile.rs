use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    error::AppError,
    middleware::auth::{AuthUser, VerifiedUser},
    models::user::{PrivateUser, User},
    response, services,
    state::AppState,
    validation,
};

pub async fn me(auth: AuthUser) -> Json<Value> {
    response::ok(PrivateUser::from(&auth.user))
}

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub username: Option<String>,
    pub nickname: Option<String>,
}

pub async fn update_profile(
    State(state): State<AppState>,
    auth: VerifiedUser,
    Json(req): Json<UpdateProfileRequest>,
) -> Result<Json<Value>, AppError> {
    if let Some(u) = &req.username {
        if !validation::username(u.trim()) {
            return Err(AppError::bad_request(
                "Usernames are 3-30 characters: letters, numbers, dots and underscores only.",
            ));
        }
    }
    if let Some(n) = &req.nickname {
        if !validation::nickname(n) {
            return Err(AppError::bad_request("Nicknames are 1-40 characters."));
        }
    }

    state.check_write(auth.user.id)?;

    let result = sqlx::query(
        "UPDATE users
         SET username = COALESCE($2, username),
             nickname = COALESCE($3, nickname),
             updated_at = now()
         WHERE id = $1",
    )
    .bind(auth.user.id)
    .bind(req.username.as_deref().map(str::trim))
    .bind(req.nickname.as_deref().map(str::trim))
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(AppError::conflict("That username is already taken. Try another?"));
        }
        Err(e) => return Err(e.into()),
    }

    let user: User = services::user::by_id(&state, auth.user.id).await?;
    Ok(response::ok(PrivateUser::from(&user)))
}

pub async fn get_public_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<crate::handlers::sightings::PageQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = crate::paging::limit(q.limit);
    let cursor = crate::paging::decode(q.cursor.as_deref())?;
    let profile = services::user::profile(&state, id, limit, cursor).await?;
    Ok(response::ok(profile))
}

pub async fn upload_avatar(
    State(state): State<AppState>,
    auth: VerifiedUser,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    state.check_write(auth.user.id)?;
    let max_bytes = state.config.avatar.max_bytes;
    let mut data: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("Invalid upload form: {e}")))?
    {
        if field.name() == Some("avatar") {
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
                    "Profile pictures have to be under {mb} MB."
                )));
            }
            data = Some(bytes.to_vec());
            break;
        }
    }

    let data = data.ok_or_else(|| {
        AppError::bad_request("No image found. Send it as the 'avatar' field.")
    })?;

    // Decoding + re-encoding is CPU-bound; keep it off the async runtime.
    let max_dim = state.config.avatar.max_dimension;
    let quality = state.config.avatar.webp_quality;
    let webp_bytes = tokio::task::spawn_blocking(move || {
        services::image::to_webp(&data, max_dim, quality)
    })
    .await
    .map_err(|e| AppError::internal(format!("image task failed: {e}")))??;

    let dir = &state.config.upload_dir;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    // Content-hashed filename: a new upload is a new URL, so neither Cloudflare nor the
    // client's image cache can keep serving the previous picture.
    let hash = hex::encode(Sha256::digest(&webp_bytes));
    let filename = format!("{}-{}.webp", auth.user.id, &hash[..16]);

    // Remember the old URL so we can drop the previous file after the row moves on.
    let previous: Option<String> = sqlx::query_scalar("SELECT avatar_url FROM users WHERE id = $1")
        .bind(auth.user.id)
        .fetch_one(&state.db)
        .await?;

    tokio::fs::write(dir.join(&filename), &webp_bytes)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    let url = format!("{}/avatars/{filename}", state.config.public_base_url);
    sqlx::query("UPDATE users SET avatar_url = $2, updated_at = now() WHERE id = $1")
        .bind(auth.user.id)
        .bind(&url)
        .execute(&state.db)
        .await?;

    // Best-effort: remove the previous file, and the legacy stable-name one if present.
    if let Some(old) = previous.and_then(|u| u.rsplit('/').next().map(str::to_owned)) {
        let _ = tokio::fs::remove_file(dir.join(&old)).await;
    }
    let _ = tokio::fs::remove_file(dir.join(format!("{}.webp", auth.user.id))).await;

    let user: User = services::user::by_id(&state, auth.user.id).await?;
    Ok(response::ok(PrivateUser::from(&user)))
}

pub async fn delete_avatar(
    State(state): State<AppState>,
    auth: VerifiedUser,
) -> Result<Json<Value>, AppError> {
    // Drop the file first; the row update is what actually makes it disappear, so a
    // missing file (already gone) is fine.
    let previous: Option<String> = sqlx::query_scalar("SELECT avatar_url FROM users WHERE id = $1")
        .bind(auth.user.id)
        .fetch_one(&state.db)
        .await?;
    if let Some(name) = previous.and_then(|u| u.rsplit('/').next().map(str::to_owned)) {
        let _ = tokio::fs::remove_file(state.config.upload_dir.join(name)).await;
    }
    // Also clear any legacy stable-name file from before the content-hashed scheme.
    let _ = tokio::fs::remove_file(
        state
            .config
            .upload_dir
            .join(format!("{}.webp", auth.user.id)),
    )
    .await;

    sqlx::query("UPDATE users SET avatar_url = NULL, updated_at = now() WHERE id = $1")
        .bind(auth.user.id)
        .execute(&state.db)
        .await?;

    let user: User = services::user::by_id(&state, auth.user.id).await?;
    Ok(response::ok(PrivateUser::from(&user)))
}
