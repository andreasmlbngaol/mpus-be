use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::error::AppError;

/// The bits of a Google service-account key we actually need to mint an OAuth2 token.
#[derive(Debug, serde::Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

#[derive(Serialize)]
struct Claims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// Firebase Cloud Messaging sender (HTTP v1). Holds the parsed service account and a
/// cached OAuth2 access token — the token lasts an hour, so we only refresh when it's
/// near expiry rather than on every send.
pub struct Push {
    account: ServiceAccount,
    access: Mutex<Option<(String, Instant)>>,
}

impl Push {
    /// Build from the service-account JSON blob. `None` when push is not configured, so
    /// the caller can skip the field entirely.
    pub fn from_config(config: &crate::config::PushConfig) -> Result<Option<Self>, AppError> {
        let Some(raw) = &config.service_account_json else {
            return Ok(None);
        };
        let account: ServiceAccount = serde_json::from_str(raw)
            .map_err(|e| AppError::internal(format!("bad FCM service account: {e}")))?;
        Ok(Some(Self { account, access: Mutex::new(None) }))
    }

    /// The current OAuth2 access token, minted from the service account when the cached
    /// one is missing or within 60 s of expiry.
    async fn access_token(&self, http: &reqwest::Client) -> Result<String, AppError> {
        let mut guard = self.access.lock().await;
        if let Some((token, expires)) = guard.as_ref()
            && Instant::now() + Duration::from_secs(60) < *expires
        {
            return Ok(token.clone());
        }

        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            iss: self.account.client_email.clone(),
            scope: "https://www.googleapis.com/auth/firebase.messaging".to_string(),
            aud: self.account.token_uri.clone(),
            iat: now,
            exp: now + 3600,
        };
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(self.account.private_key.as_bytes())
            .map_err(|e| AppError::internal(format!("bad FCM private key: {e}")))?;
        let assertion = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &key,
        )
        .map_err(|e| AppError::internal(format!("failed to sign FCM assertion: {e}")))?;

        let resp: Value = http
            .post(&self.account.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &assertion),
            ])
            .send()
            .await?
            .json()
            .await?;

        let token = resp["access_token"]
            .as_str()
            .ok_or_else(|| AppError::internal("FCM token exchange returned no access_token"))?
            .to_string();
        let ttl = resp["expires_in"].as_i64().unwrap_or(3600).max(60) as u64;
        *guard = Some((token.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(token)
    }

    /// Send one notification to a single device token. Best-effort: an invalid/expired
    /// token is reported back so the caller can drop it, but nothing panics.
    pub async fn send(
        &self,
        http: &reqwest::Client,
        device_token: &str,
        title: &str,
        body: &str,
        data: Value,
    ) -> Result<(), PushError> {
        let access = self.access_token(http).await?;
        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.account.project_id
        );
        let message = json!({
            "message": {
                "token": device_token,
                "notification": { "title": title, "body": body },
                "data": data,
                "android": { "priority": "HIGH" },
            }
        });
        let resp = http
            .post(&url)
            .bearer_auth(access)
            .json(&message)
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // 404 UNREGISTERED / 400 INVALID_ARGUMENT on the token means it's dead — the
        // caller should forget it. Anything else is transient and worth retrying later.
        let dead = status == reqwest::StatusCode::NOT_FOUND
            || text.contains("UNREGISTERED")
            || text.contains("INVALID_ARGUMENT");
        tracing::warn!(%status, %text, "FCM rejected a push");
        if dead { Err(PushError::Dead) } else { Err(PushError::Transient) }
    }
}

#[derive(Debug)]
pub enum PushError {
    /// The device token is no longer valid; drop it.
    Dead,
    /// Anything else (network, auth, quota) — keep the token and try again later.
    Transient,
}

impl From<AppError> for PushError {
    fn from(_: AppError) -> Self {
        PushError::Transient
    }
}

impl From<reqwest::Error> for PushError {
    fn from(_: reqwest::Error) -> Self {
        PushError::Transient
    }
}
