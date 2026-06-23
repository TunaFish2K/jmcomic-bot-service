use std::sync::Arc;

use jmcomic_bot_service::{
    config::Config,
    db::Db,
    jobs::{JobQueue, spawn_workers},
    routes::{AppState, router},
    worker_client::WorkerClient,
};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("jmcomic_bot_service=info,tower_http=info,axum=info")
        }))
        .init();

    let config = Arc::new(Config::from_env()?);
    config.ensure_dirs().await?;

    let db = Db::connect(&config.database_url).await?;
    db.init().await?;

    let worker = WorkerClient::new(config.worker_base_url.clone())?;
    let queue = JobQueue::new(config.max_concurrent_jobs);

    let state = AppState {
        config: config.clone(),
        db: db.clone(),
        worker,
        queue: queue.clone(),
    };

    spawn_workers(queue, state.clone());

    let listener = TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "listening");

    axum::serve(
        listener,
        router(state)
            .layer(TraceLayer::new_for_http())
            .layer(CorsLayer::permissive()),
    )
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
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
