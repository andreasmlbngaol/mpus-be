use std::{env, path::PathBuf, str::FromStr, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    Missing(String),
    #[error("environment variable {name} has an invalid value: {value:?}")]
    Invalid { name: &'static str, value: String },
}

/// Everything tunable lives here, sourced from the environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub public_base_url: String,
    pub resend_api_key: String,
    pub mail_from: String,
    pub upload_dir: PathBuf,
    pub session_ttl_days: i64,
    pub db_max_connections: u32,
    pub avatar: AvatarConfig,
    pub password: PasswordConfig,
    pub tokens: TokenConfig,
    pub rate: RateConfig,
    pub embed: EmbedConfig,
    pub sighting: SightingConfig,
    pub push: PushConfig,
}

#[derive(Debug, Clone)]
pub struct AvatarConfig {
    pub max_bytes: usize,
    pub max_dimension: u32,
    pub webp_quality: f32,
}

#[derive(Debug, Clone)]
pub struct PasswordConfig {
    pub memory_kib: u32,
    pub time_cost: u32,
    pub parallelism: u32,
}

#[derive(Debug, Clone)]
pub struct TokenConfig {
    pub verify_ttl: Duration,
    pub reset_ttl: Duration,
}

#[derive(Debug, Clone)]
pub struct RateConfig {
    pub signup_max: u32,
    pub signup_window: Duration,
    pub login_max: u32,
    pub login_window: Duration,
    pub resend_max: u32,
    pub resend_window: Duration,
    pub forgot_max: u32,
    pub forgot_window: Duration,
    pub sighting_max: u32,
    pub sighting_window: Duration,
    /// Shared budget for the light write actions (names, reviews, likes, profile edits).
    pub write_max: u32,
    pub write_window: Duration,
}

/// The cat-identification model and how it runs.
#[derive(Debug, Clone)]
pub struct EmbedConfig {
    pub model_path: PathBuf,
    pub threads: usize,
    pub memory_arena: bool,
    /// How many similar cats to offer as candidates (never auto-linked).
    pub top_k: u32,
    /// Minimum cosine similarity for a cat to count as a candidate at all. Below this
    /// the "match" is noise, and offering it would claim two random cats are the same.
    pub min_similarity: f32,
}

/// Upload limits for cat photos.
#[derive(Debug, Clone)]
pub struct SightingConfig {
    pub max_bytes: usize,
    pub max_dimension: u32,
    pub webp_quality: f32,
    /// Longest side of the small WebP used as a map marker thumbnail.
    pub thumb_max_dim: u32,
}

/// Push notifications through Firebase Cloud Messaging (HTTP v1).
///
/// Optional as a whole: with no `FCM_SERVICE_ACCOUNT_JSON` the app simply never pushes,
/// and the in-app inbox keeps working. The service account key is a JSON blob (either
/// raw JSON or a path to the file); it is a secret and lives only in the server's `.env`.
#[derive(Debug, Clone, Default)]
pub struct PushConfig {
    pub service_account_json: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            host: optional("HOST", "127.0.0.1"),
            port: parsed("PORT", 8080)?,
            database_url: required("DATABASE_URL")?,
            public_base_url: required("PUBLIC_BASE_URL")?,
            resend_api_key: required("RESEND_API_KEY")?,
            mail_from: required("MAIL_FROM")?,
            upload_dir: PathBuf::from(optional("UPLOAD_DIR", "./uploads")),
            session_ttl_days: parsed("SESSION_TTL_DAYS", 60)?,
            db_max_connections: parsed("DB_MAX_CONNECTIONS", 10)?,
            avatar: AvatarConfig {
                max_bytes: parsed("AVATAR_MAX_BYTES", 5 * 1024 * 1024)?,
                max_dimension: parsed("AVATAR_MAX_DIMENSION", 6000)?,
                webp_quality: parsed("AVATAR_WEBP_QUALITY", 90.0)?,
            },
            password: PasswordConfig {
                memory_kib: parsed("ARGON2_MEMORY_KIB", 12 * 1024)?,
                time_cost: parsed("ARGON2_TIME_COST", 3)?,
                parallelism: parsed("ARGON2_PARALLELISM", 1)?,
            },
            tokens: TokenConfig {
                verify_ttl: Duration::from_secs(parsed("VERIFY_TOKEN_TTL_SECS", 86_400)?),
                reset_ttl: Duration::from_secs(parsed("RESET_TOKEN_TTL_SECS", 3_600)?),
            },
            rate: RateConfig {
                signup_max: parsed("RATE_SIGNUP_MAX", 5)?,
                signup_window: Duration::from_secs(parsed("RATE_SIGNUP_WINDOW_SECS", 3_600)?),
                login_max: parsed("RATE_LOGIN_MAX", 10)?,
                login_window: Duration::from_secs(parsed("RATE_LOGIN_WINDOW_SECS", 900)?),
                resend_max: parsed("RATE_RESEND_MAX", 3)?,
                resend_window: Duration::from_secs(parsed("RATE_RESEND_WINDOW_SECS", 3_600)?),
                forgot_max: parsed("RATE_FORGOT_MAX", 5)?,
                forgot_window: Duration::from_secs(parsed("RATE_FORGOT_WINDOW_SECS", 3_600)?),
                sighting_max: parsed("RATE_SIGHTING_MAX", 30)?,
                sighting_window: Duration::from_secs(parsed("RATE_SIGHTING_WINDOW_SECS", 3_600)?),
                write_max: parsed("RATE_WRITE_MAX", 60)?,
                write_window: Duration::from_secs(parsed("RATE_WRITE_WINDOW_SECS", 600)?),
            },
            embed: EmbedConfig {
                model_path: PathBuf::from(optional(
                    "EMBED_MODEL_PATH",
                    "./models/megadescriptor-t-224-int8.onnx",
                )),
                threads: parsed("EMBED_THREADS", 1)?,
                memory_arena: parsed("EMBED_ARENA", false)?,
                top_k: parsed("EMBED_TOP_K", 5)?,
                min_similarity: parsed("EMBED_MIN_SIMILARITY", 0.55)?,
            },
            sighting: SightingConfig {
                max_bytes: parsed("SIGHTING_MAX_BYTES", 10 * 1024 * 1024)?,
                max_dimension: parsed("SIGHTING_MAX_DIMENSION", 8000)?,
                webp_quality: parsed("SIGHTING_WEBP_QUALITY", 90.0)?,
                thumb_max_dim: parsed("SIGHTING_THUMB_MAX_DIM", 512)?,
            },
            push: PushConfig {
                // Accept either the raw JSON blob or a path to the key file.
                service_account_json: match optional("FCM_SERVICE_ACCOUNT_JSON", "") {
                    s if s.trim().is_empty() => None,
                    s if s.trim_start().starts_with('{') => Some(s),
                    s => std::fs::read_to_string(s.trim()).ok(),
                },
            },
        })
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn required(key: &'static str) -> Result<String, ConfigError> {
    env::var(key).map_err(|_| ConfigError::Missing(key.to_string()))
}

fn optional(key: &'static str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn parsed<T: FromStr>(key: &'static str, default: T) -> Result<T, ConfigError> {
    match env::var(key) {
        Ok(v) => v.trim().parse::<T>().map_err(|_| ConfigError::Invalid {
            name: key,
            value: v,
        }),
        Err(_) => Ok(default),
    }
}
