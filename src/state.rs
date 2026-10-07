use std::sync::Arc;

use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use crate::{
    auth,
    config::Config,
    error::AppError,
    ratelimit::RateLimiter,
    services::embedding::EmbeddingService,
    services::push::Push,
};

/// Shared, cheaply-cloneable application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
    pub rate: Arc<RateLimiter>,
    /// The cat-identification model, loaded once.
    pub embed: Arc<EmbeddingService>,
    /// FCM sender, or `None` when push isn't configured.
    pub push: Option<Arc<Push>>,
    /// A valid argon2 hash used to equalize login timing when the email is
    /// unknown (prevents user enumeration via response time).
    pub dummy_password_hash: Arc<str>,
}

impl AppState {
    pub async fn init(config: Config) -> Result<Self, Box<dyn std::error::Error>> {
        let db = PgPoolOptions::new()
            .max_connections(config.db_max_connections)
            .connect(&config.database_url)
            .await?;

        sqlx::migrate!("./migrations").run(&db).await?;
        tokio::fs::create_dir_all(&config.upload_dir).await?;

        // Load the embedding model up front so the first upload isn't slow.
        let embed = EmbeddingService::load(&config.embed)?;

        // Hash a random secret so the dummy matches the configured cost params.
        let dummy_password_hash: Arc<str> =
            auth::password::hash(&config.password, &auth::tokens::random_token())?.into();

        let push = Push::from_config(&config.push)?.map(Arc::new);
        if push.is_some() {
            tracing::info!("FCM push enabled");
        }

        Ok(Self {
            db,
            config: Arc::new(config),
            http: reqwest::Client::new(),
            rate: Arc::new(RateLimiter::new()),
            embed: Arc::new(embed),
            push,
            dummy_password_hash,
        })
    }

    /// Per-user budget for the light write actions (names, reviews, likes, profile
    /// edits). One shared window keeps the endpoints consistent: `Err` if the caller
    /// has burned through `write_max` in `write_window`.
    pub fn check_write(&self, user_id: Uuid) -> Result<(), AppError> {
        let r = &self.config.rate;
        if !self
            .rate
            .check(&crate::ratelimit::key_str(&user_id.to_string(), "write"), r.write_max, r.write_window)
        {
            return Err(AppError::too_many(
                "Slow down a moment — you're patting the cats too fast.",
            ));
        }
        Ok(())
    }
}
