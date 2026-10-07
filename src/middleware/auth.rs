use axum::{
    extract::{FromRequestParts, OptionalFromRequestParts},
    http::{header::AUTHORIZATION, request::Parts},
};

use crate::{auth::tokens, error::AppError, models::user::User, state::AppState};

/// Extractor for routes that need a logged-in user.
///
/// Reads `Authorization: Bearer <token>`, resolves the session, and hands the
/// handler the user plus the session's token hash (so logout can revoke it).
pub struct AuthUser {
    pub user: User,
    pub token_hash: String,
}

impl AuthUser {
    /// Resolve a bearer token into an `AuthUser`, or `None` if there's no token
    /// or it doesn't match a live session. Used by routes that work signed-out
    /// but personalize when signed in.
    async fn resolve(parts: &Parts, state: &AppState) -> Option<Self> {
        let token = parts
            .headers
            .get(AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?;

        let token_hash = tokens::hash_token(token);
        let user = sqlx::query_as::<_, User>(
            "SELECT u.id, u.email, u.email_verified_at, u.password_hash,
                    u.username, u.nickname, u.avatar_url, u.created_at, u.updated_at
             FROM sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(&token_hash)
        .fetch_optional(&state.db)
        .await
        .ok()??;

        touch_session(state, &token_hash);
        Some(AuthUser { user, token_hash })
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if parts.headers.get(AUTHORIZATION).is_none() {
            return Err(AppError::unauthorized("You need to sign in first."));
        }
        Self::resolve(parts, state)
            .await
            .ok_or_else(|| AppError::unauthorized("That session has expired. Please sign in again."))
    }
}

impl OptionalFromRequestParts<AppState> for AuthUser {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(Self::resolve(parts, state).await)
    }
}

/// Like [AuthUser], but only for a *confirmed* account. An unverified user can hold a
/// session (so the client can show the verify screen) yet must not touch any write
/// endpoint — the client-side gate is a nicety, this is the actual lock. Reads stay on
/// [AuthUser]: an unverified account may browse while it waits.
pub struct VerifiedUser {
    pub user: User,
    #[allow(dead_code)] // kept for parity with AuthUser; some handlers may want it
    pub token_hash: String,
}

impl FromRequestParts<AppState> for VerifiedUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let auth = <AuthUser as FromRequestParts<AppState>>::from_request_parts(parts, state).await?;
        if auth.user.email_verified_at.is_none() {
            return Err(AppError::forbidden(
                "Confirm your email first — check your inbox for the code.",
            ));
        }
        Ok(VerifiedUser { user: auth.user, token_hash: auth.token_hash })
    }
}

/// Update `last_used_at` without blocking the request.
fn touch_session(state: &AppState, token_hash: &str) {
    let db = state.db.clone();
    let th = token_hash.to_string();
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE sessions SET last_used_at = now() WHERE token_hash = $1")
            .bind(&th)
            .execute(&db)
            .await;
    });
}
