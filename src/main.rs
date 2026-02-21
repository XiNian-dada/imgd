use std::{sync::Arc, time::Duration};

use imgd::{
    build_app,
    config::AppConfig,
    token::{token_cli, TokenStore},
    with_connect_info, AppState, Metrics, SimpleRateLimiter,
};
use tokio::{net::TcpListener, sync::Semaphore};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "token" {
        token_cli(&args[2..])?;
        return Ok(());
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "imgd=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env()?;
    config.ensure_data_dir_ready()?;
    let token_store = TokenStore::from_config(&config)?;

    let state = AppState {
        upload_semaphore: Arc::new(Semaphore::new(config.max_concurrent_uploads)),
        rate_limiter: SimpleRateLimiter::new(Duration::from_secs(60)),
        token_store,
        metrics: Arc::new(Metrics::default()),
        config: config.clone(),
    };

    let listener = TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "imgd listening");

    axum::serve(listener, with_connect_info(build_app(state))).await?;
    Ok(())
}
