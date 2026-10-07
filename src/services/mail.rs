use serde_json::json;
use uuid::Uuid;

use crate::{auth::tokens, config::Config, error::AppError, state::AppState};

/// Mint a fresh 6-digit verify code for [user_id], store its hash, and email it.
/// Shared by signup and resend so the two paths can't drift.
pub async fn send_verification_code(
    state: &AppState,
    user_id: Uuid,
    nickname: &str,
    email: &str,
) -> Result<(), AppError> {
    let code = tokens::random_code();
    sqlx::query(
        "INSERT INTO email_tokens (user_id, kind, token_hash, expires_at)
         VALUES ($1, 'verify', $2, now() + make_interval(secs => $3))",
    )
    .bind(user_id)
    .bind(tokens::hash_token(&code))
    .bind(state.config.tokens.verify_ttl.as_secs_f64())
    .execute(&state.db)
    .await?;

    let link = format!("{}/verify-email?token={}", state.config.public_base_url, code);
    let (subject, body) = verification_email(nickname, &code, &link);
    send(&state.http, &state.config, email, &subject, body).await
}

/// Send an email through Resend.
pub async fn send(
    http: &reqwest::Client,
    config: &Config,
    to: &str,
    subject: &str,
    html: String,
) -> Result<(), AppError> {
    let resp = http
        .post("https://api.resend.com/emails")
        .bearer_auth(&config.resend_api_key)
        .json(&json!({
            "from": config.mail_from,
            "to": to,
            "subject": subject,
            "html": html,
        }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::error!(%status, %body, "resend rejected the email");
        return Err(AppError::internal("failed to send email"));
    }
    Ok(())
}

pub fn verification_email(nickname: &str, token: &str, link: &str) -> (String, String) {
    let subject = format!("{token} is your MPUS code");
    let body = format!(
        r#"<div style="font-family:sans-serif;max-width:420px">
        <p>Hey {nickname}, welcome to MPUS.</p>
        <p>Your code:</p>
        <p style="font-size:36px;font-weight:bold;letter-spacing:8px;margin:8px 0">{token}</p>
        <p style="color:#666;font-size:13px">Good for 24 hours. Or <a href="{link}">tap here to confirm</a>.</p>
        </div>"#
    );
    (subject, body)
}

pub fn reset_email(token: &str, link: &str) -> (String, String) {
    let subject = "Reset your MPUS password".to_string();
    let body = format!(
        r#"<div style="font-family:sans-serif;max-width:480px">
        <h2>Forgot your password? No worries.</h2>
        <p>Here's your reset code (it expires in 1 hour):</p>
        <p style="font-size:28px;font-weight:bold;letter-spacing:4px">{token}</p>
        <p>Or click here:<br><a href="{link}">{link}</a></p>
        <p>If you didn't ask for this, you can safely ignore it.</p>
        </div>"#
    );
    (subject, body)
}
