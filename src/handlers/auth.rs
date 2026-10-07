use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::{password, session},
    error::AppError,
    middleware::auth::AuthUser,
    models::user::{PrivateUser, User},
    ratelimit::key,
    response, services,
    state::AppState,
    validation,
};

#[derive(Deserialize)]
pub struct SignupRequest {
    pub email: String,
    pub password: String,
    pub username: String,
    pub nickname: String,
}

pub async fn signup(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SignupRequest>,
) -> Result<Json<Value>, AppError> {
    let r = &state.config.rate;
    if !state.rate.check(&key(addr.ip(), "signup"), r.signup_max, r.signup_window) {
        return Err(AppError::too_many(
            "Whoa, slow down! Too many sign-ups from here. Try again later.",
        ));
    }

    let email = validation::normalize_email(&req.email);
    let username = req.username.trim().to_string();
    let nickname = req.nickname.trim().to_string();

    if !validation::email(&email) {
        return Err(AppError::bad_request("That email address doesn't look right."));
    }
    if !validation::password(&req.password) {
        return Err(AppError::bad_request("Passwords need at least 8 characters."));
    }
    if !validation::username(&username) {
        return Err(AppError::bad_request(
            "Usernames are 3-30 characters: letters, numbers, dots and underscores only.",
        ));
    }
    if !validation::nickname(&nickname) {
        return Err(AppError::bad_request("Nicknames are 1-40 characters."));
    }

    let hash = password::hash(&state.config.password, &req.password)?;

    let inserted: Result<(Uuid,), sqlx::Error> = sqlx::query_as(
        "INSERT INTO users (email, password_hash, username, nickname)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(&email)
    .bind(&hash)
    .bind(&username)
    .bind(&nickname)
    .fetch_one(&state.db)
    .await;

    let (user_id,) = match inserted {
        Ok(r) => r,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            let constraint = e.constraint().unwrap_or("");
            if constraint.contains("username") {
                return Err(AppError::conflict("That username is already taken. Try another?"));
            }
            return Err(AppError::conflict("That email is already registered. Try signing in?"));
        }
        Err(e) => return Err(e.into()),
    };

    if let Err(e) = services::mail::send_verification_code(&state, user_id, &nickname, &email).await {
        // No email, no account: roll back so we don't strand an unverifiable user.
        tracing::error!(error = %e, "verification email failed; rolling back signup");
        let _ = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&state.db)
            .await;
        return Err(AppError::internal(
            "We couldn't send the verification email. Please try again.",
        ));
    }

    // Sign them in straight away so the app can drop them on the verify screen instead
    // of bouncing back to a login form they just filled in. The account stays gated on
    // the client until the code is entered.
    let session_token = session::create(&state, user_id, user_agent(&headers)).await?;
    let user: User = services::user::by_id(&state, user_id).await?;

    Ok(response::ok(json!({
        "token": session_token,
        "user": PrivateUser::from(&user),
    })))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<Value>, AppError> {
    let r = &state.config.rate;
    if !state.rate.check(&key(addr.ip(), "login"), r.login_max, r.login_window) {
        return Err(AppError::too_many(
            "Too many login attempts. Take a breather and try again.",
        ));
    }

    let email = validation::normalize_email(&req.email);
    let row: Option<(Uuid, String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as("SELECT id, password_hash, email_verified_at FROM users WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;

    // Always run argon2, even when the email is unknown, so response time
    // doesn't reveal which emails are registered (timing attack).
    let hash = row
        .as_ref()
        .map(|(_, h, _)| h.as_str())
        .unwrap_or(&state.dummy_password_hash);
    let password_ok = password::verify(&state.config.password, &req.password, hash).unwrap_or(false);

    let Some((user_id, _, _)) = row else {
        return Err(AppError::unauthorized("Wrong email or password."));
    };
    if !password_ok {
        return Err(AppError::unauthorized("Wrong email or password."));
    }
    // An unverified account is allowed through: the client holds it on the verify
    // screen (it sees `email_verified: false` on the user) instead of walling it off.
    // The in-app inbox keeps working meanwhile.

    let token = session::create(&state, user_id, user_agent(&headers)).await?;
    let user: User = services::user::by_id(&state, user_id).await?;

    Ok(response::ok(json!({
        "token": token,
        "user": PrivateUser::from(&user),
    })))
}

pub async fn logout(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, AppError> {
    session::revoke(&state, &auth.token_hash).await?;
    Ok(response::message("Signed out. See you soon!"))
}

#[derive(Deserialize)]
pub struct DeviceTokenRequest {
    pub token: String,
    #[serde(default = "default_platform")]
    pub platform: String,
}

fn default_platform() -> String {
    "android".to_string()
}

/// `POST /devices` — register (or refresh) this device's FCM token for the caller.
pub async fn register_device(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<DeviceTokenRequest>,
) -> Result<Json<Value>, AppError> {
    let token = req.token.trim();
    if token.is_empty() || token.len() > 4096 {
        return Err(AppError::bad_request("That device token doesn't look right."));
    }
    services::notification::register_device(&state, auth.user.id, token, &req.platform).await?;
    Ok(response::message("Device registered."))
}

/// `POST /devices/delete` — forget this device's token (called on sign-out).
pub async fn unregister_device(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<DeviceTokenRequest>,
) -> Result<Json<Value>, AppError> {
    services::notification::unregister_device(&state, req.token.trim()).await?;
    Ok(response::message("Device removed."))
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(300).collect())
}
