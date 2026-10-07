use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// A window into a longer list. Keyset (cursor) paging: the client passes the
/// `next_cursor` back to fetch the following slice; `next_cursor` is `None` on the
/// last page.
#[derive(Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// `GET /me/sightings` — a page of your uploads plus the totals the "You" tab shows.
/// The counts are exact server-side aggregates; the grid only holds one page at a time.
#[derive(Serialize)]
pub struct MySightings {
    pub items: Vec<SightingView>,
    pub next_cursor: Option<String>,
    /// Every photo you've uploaded.
    pub sightings_count: i64,
    /// Distinct cats those photos are attached to.
    pub cats_count: i64,
    /// Photos still pending — not yet linked or named.
    pub unnamed_count: i64,
}

/// A cat: an identity that many sightings can point at.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)] // fields exist for FromRow; not all are read yet
pub struct Cat {
    pub id: Uuid,
    pub created_by: Option<Uuid>,
    /// Denormalized winner (most-liked name); drives the map label.
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One photo of a cat, with where it was taken and the model's embedding.
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)] // fields exist for FromRow; not all are read yet
pub struct Sighting {
    pub id: Uuid,
    pub user_id: Uuid,
    /// `None` = pending: the uploader hasn't decided link-vs-new yet.
    pub cat_id: Option<Uuid>,
    pub photo_url: String,
    pub thumb_url: String,
    pub lat: f64,
    pub lng: f64,
    pub taken_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// What the uploader sees right after a photo: their sighting plus similar cats.
#[derive(Serialize)]
pub struct SightingCreated {
    pub sighting: SightingView,
    pub candidates: Vec<Candidate>,
}

/// A possible existing match. Never auto-linked — the user decides.
#[derive(Serialize)]
pub struct Candidate {
    pub cat_id: Uuid,
    pub thumb_url: String,
    pub display_name: Option<String>,
    /// Cosine similarity in [0, 1]; 1.0 means identical.
    pub similarity: f32,
}

/// A sighting as sent to clients (no raw embedding).
#[derive(Serialize)]
pub struct SightingView {
    pub id: Uuid,
    pub cat_id: Option<Uuid>,
    pub photo_url: String,
    pub thumb_url: String,
    pub lat: f64,
    pub lng: f64,
    pub taken_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<Sighting> for SightingView {
    fn from(s: Sighting) -> Self {
        Self {
            id: s.id,
            cat_id: s.cat_id,
            photo_url: s.photo_url,
            thumb_url: s.thumb_url,
            lat: s.lat,
            lng: s.lng,
            taken_at: s.taken_at,
            created_at: s.created_at,
        }
    }
}

/// A cat marker on the map (position = its most recent sighting).
#[derive(Serialize, sqlx::FromRow)]
pub struct CatMarker {
    pub id: Uuid,
    pub display_name: Option<String>,
    pub thumb_url: String,
    pub lat: f64,
    pub lng: f64,
    pub sighting_count: i64,
}

/// A name plus its like tally, from one viewer's perspective.
#[derive(Serialize, sqlx::FromRow)]
pub struct NameView {
    pub id: Uuid,
    pub user_id: Uuid,
    pub nickname: String,
    pub name: String,
    pub likes: i64,
    pub liked_by_me: bool,
}

/// A review plus its like tally, from one viewer's perspective.
#[derive(Serialize, sqlx::FromRow)]
pub struct ReviewView {
    pub id: Uuid,
    pub user_id: Uuid,
    pub nickname: String,
    pub body: String,
    pub rating: i16,
    pub likes: i64,
    pub liked_by_me: bool,
    pub created_at: DateTime<Utc>,
}

/// Full cat detail: names (ranked), reviews, and every sighting.
#[derive(Serialize)]
pub struct CatDetail {
    pub id: Uuid,
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub names: Vec<NameView>,
    pub reviews: Vec<ReviewView>,
    pub sightings: Vec<SightingView>,
    /// Whether the viewer has photographed this cat. Naming and reviewing are gated on it.
    pub can_contribute: bool,
}
