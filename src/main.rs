mod auth;
mod config;
mod error;
mod handlers;
mod middleware;
mod models;
mod paging;
mod ratelimit;
mod response;
mod router;
mod services;
mod state;
mod validation;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mpus=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    let addr = config.bind_addr();

    let state = AppState::init(config).await?;
    let app = router::build(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("MPUS is listening on {addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}
