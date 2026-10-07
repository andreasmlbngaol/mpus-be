use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// The full user row as stored.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)] // password_hash / timestamps are needed by FromRow, not always read
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub password_hash: String,
    pub username: String,
    pub nickname: String,
    pub avatar_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What the account owner sees.
#[derive(Serialize)]
pub struct PrivateUser {
    pub id: Uuid,
    pub email: String,
    pub email_verified: bool,
    pub username: String,
    pub nickname: String,
    pub avatar_url: Option<String>,
}

impl From<&User> for PrivateUser {
    fn from(u: &User) -> Self {
        Self {
            id: u.id,
            email: u.email.clone(),
            email_verified: u.email_verified_at.is_some(),
            username: u.username.clone(),
            nickname: u.nickname.clone(),
            avatar_url: u.avatar_url.clone(),
        }
    }
}

/// A public profile page: who they are plus what they've contributed. Drives the
/// "tap a name in the leaderboard, see the person behind it" screen.
#[derive(Serialize)]
pub struct UserProfile {
    pub id: Uuid,
    pub username: String,
    pub nickname: String,
    pub avatar_url: Option<String>,
    /// Photos they've uploaded.
    pub sightings_count: i64,
    /// Distinct cats their photos are attached to — cats they've "found".
    pub cats_count: i64,
    /// Names they've given.
    pub names_count: i64,
    /// The cats they've photographed, keyset-paged newest-first.
    pub cats: crate::models::cat::Page<crate::models::cat::CatMarker>,
}
