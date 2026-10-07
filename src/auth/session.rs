use uuid::Uuid;

use crate::{auth::tokens, error::AppError, state::AppState};

/// Create a new opaque session and return the raw token to hand to the client.
/// Only the hash is persisted.
pub async fn create(
    state: &AppState,
    user_id: Uuid,
    user_agent: Option<String>,
) -> Result<String, AppError> {
    let token = tokens::random_token();
    let token_hash = tokens::hash_token(&token);
    let expires = chrono::Utc::now() + chrono::Duration::days(state.config.session_ttl_days);

    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, expires_at, user_agent)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(&token_hash)
    .bind(expires)
    .bind(user_agent)
    .execute(&state.db)
    .await?;

    Ok(token)
}

/// Revoke a single session by its token hash.
pub async fn revoke(state: &AppState, token_hash: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(&state.db)
        .await?;
    Ok(())
}

/// Revoke every session for a user (e.g. after a password reset).
pub async fn revoke_all(state: &AppState, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(())
}
