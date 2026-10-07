use axum::{
    Json,
    extract::{ConnectInfo, State},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    auth::{password, session, tokens},
    error::AppError,
    ratelimit::key,
    response, services,
    state::AppState,
    validation,
};

#[derive(Deserialize)]
pub struct TokenRequest {
    pub token: String,
}

pub async fn verify_email(
    State(state): State<AppState>,
    Json(req): Json<TokenRequest>,
) -> Result<Json<Value>, AppError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE email_tokens SET used_at = now()
         WHERE token_hash = $1 AND kind = 'verify'
           AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(tokens::hash_token(&req.token))
    .fetch_optional(&state.db)
    .await?;

    let (user_id,) = row.ok_or_else(|| {
        AppError::bad_request("That verification code is invalid or has expired.")
    })?;

    sqlx::query("UPDATE users SET email_verified_at = now(), updated_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await?;

    // Hand back the updated user so the client can refresh its cached session and let the
    // account past the verify gate without a round-trip through /me.
    let user = services::user::by_id(&state, user_id).await?;
    Ok(response::ok(crate::models::user::PrivateUser::from(&user)))
}

#[derive(Deserialize)]
pub struct EmailRequest {
    pub email: String,
}

pub async fn resend_verification(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<EmailRequest>,
) -> Result<Json<Value>, AppError> {
    let r = &state.config.rate;
    if !state.rate.check(&key(addr.ip(), "resend"), r.resend_max, r.resend_window) {
        return Err(AppError::too_many("Too many requests. Give it a minute."));
    }

    let email = validation::normalize_email(&req.email);
    let row: Option<(Uuid, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as("SELECT id, email_verified_at FROM users WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;

    // Generic response either way — don't reveal whether the email exists.
    if let Some((user_id, None)) = row {
        let _ = services::mail::send_verification_code(&state, user_id, "there", &email).await;
    }

    Ok(response::message(
        "If that email is registered and unverified, a new code is on its way.",
    ))
}

pub async fn forgot_password(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<EmailRequest>,
) -> Result<Json<Value>, AppError> {
    let r = &state.config.rate;
    if !state.rate.check(&key(addr.ip(), "forgot"), r.forgot_max, r.forgot_window) {
        return Err(AppError::too_many("Too many requests. Give it a minute."));
    }

    let email = validation::normalize_email(&req.email);
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&email)
        .fetch_optional(&state.db)
        .await?;

    if let Some((user_id,)) = row {
        let token = tokens::random_token();
        sqlx::query(
            "INSERT INTO email_tokens (user_id, kind, token_hash, expires_at)
             VALUES ($1, 'reset', $2, now() + make_interval(secs => $3))",
        )
        .bind(user_id)
        .bind(tokens::hash_token(&token))
        .bind(state.config.tokens.reset_ttl.as_secs_f64())
        .execute(&state.db)
        .await?;

        let link = format!("{}/reset-password?token={}", state.config.public_base_url, token);
        let (subject, body) = services::mail::reset_email(&token, &link);
        let _ = services::mail::send(&state.http, &state.config, &email, &subject, body).await;
    }

    Ok(response::message(
        "If that email is registered, a reset code is on its way.",
    ))
}

#[derive(Deserialize)]
pub struct ResetRequest {
    pub token: String,
    pub new_password: String,
}

pub async fn reset_password(
    State(state): State<AppState>,
    Json(req): Json<ResetRequest>,
) -> Result<Json<Value>, AppError> {
    if !validation::password(&req.new_password) {
        return Err(AppError::bad_request("Passwords need at least 8 characters."));
    }

    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE email_tokens SET used_at = now()
         WHERE token_hash = $1 AND kind = 'reset'
           AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(tokens::hash_token(&req.token))
    .fetch_optional(&state.db)
    .await?;

    let (user_id,) =
        row.ok_or_else(|| AppError::bad_request("That reset code is invalid or has expired."))?;

    let hash = password::hash(&state.config.password, &req.new_password)?;
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(&hash)
        .execute(&state.db)
        .await?;

    // A password change invalidates every existing session.
    session::revoke_all(&state, user_id).await?;

    Ok(response::message("Password updated! Sign in with the new one."))
}
