use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post},
};
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::{handlers, state::AppState};

pub fn build(state: AppState) -> Router {
    let upload_dir = state.config.upload_dir.clone();
    // Multipart overhead sits on top of the file itself; give it headroom.
    let body_limit = state
        .config
        .sighting
        .max_bytes
        .max(state.config.avatar.max_bytes)
        + 1024 * 1024;

    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/auth/signup", post(handlers::auth::signup))
        .route("/auth/login", post(handlers::auth::login))
        .route("/auth/logout", post(handlers::auth::logout))
        .route("/devices", post(handlers::auth::register_device))
        .route("/devices/delete", post(handlers::auth::unregister_device))
        .route("/auth/verify-email", post(handlers::email::verify_email))
        .route(
            "/auth/resend-verification",
            post(handlers::email::resend_verification),
        )
        .route(
            "/auth/forgot-password",
            post(handlers::email::forgot_password),
        )
        .route(
            "/auth/reset-password",
            post(handlers::email::reset_password),
        )
        .route("/me", get(handlers::profile::me))
        .route("/me", patch(handlers::profile::update_profile))
        .route("/me/avatar", post(handlers::profile::upload_avatar))
        .route("/me/avatar", delete(handlers::profile::delete_avatar))
        .route("/me/sightings", get(handlers::sightings::mine))
        .route("/users/{id}", get(handlers::profile::get_public_user))
        .route("/sightings", post(handlers::sightings::create))
        .route("/sightings/{id}/resolve", post(handlers::sightings::resolve))
        .route("/cats", get(handlers::cats::list))
        .route("/cats/{id}", get(handlers::cats::detail))
        .route("/cats/{id}/names", post(handlers::cats::set_name))
        .route("/cats/{id}/reviews", post(handlers::cats::set_review))
        .route("/cats/{id}/merge", post(handlers::merge::request))
        .route("/names/{id}/like", post(handlers::cats::like))
        .route("/reviews/{id}/like", post(handlers::cats::like_review))
        .route("/reports", post(handlers::moderation::create))
        .route("/merges/pending", get(handlers::merge::pending))
        .route("/merges/{id}/approve", post(handlers::merge::approve))
        .route("/merges/{id}/reject", post(handlers::merge::reject))
        .route("/merges/{id}/undo", post(handlers::merge::undo))
        .route("/notifications", get(handlers::notifications::list))
        .route(
            "/notifications/unread-count",
            get(handlers::notifications::unread),
        )
        .route("/notifications/seen", post(handlers::notifications::seen))
        .nest_service("/avatars", ServeDir::new(upload_dir))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
