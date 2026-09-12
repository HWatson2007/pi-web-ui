mod catalog;
mod config;
mod hub;
mod protocol;
mod rpc;
mod web;

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use clap::Parser;
use config::Config;
use hub::SessionHub;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;
use web::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("pi_mobile_ui=info")),
        )
        .with_target(false)
        .compact()
        .init();

    let config = Config::parse().resolve()?;
    let address = (config.host, config.port);
    let hub = Arc::new(SessionHub::new(config.clone()));
    let app = web::router(AppState {
        hub: Arc::clone(&hub),
    });
    let listener = TcpListener::bind(address).await?;
    info!(host = %config.host, port = config.port, cwd = %config.cwd().display(), "Pi Mobile UI is ready");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        signal(SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    tokio::time::sleep(Duration::from_millis(100)).await;
}
