use std::sync::Arc;
use std::{env, path::PathBuf};

use anyhow::{Context, bail};
use jmcomic_bot_service::{
    config::{Config, DEFAULT_CONFIG_PATH},
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

    let Some(config_path) = parse_config_path()? else {
        print_usage();
        return Ok(());
    };

    let config = Arc::new(Config::from_file(&config_path)?);
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
    tracing::info!(addr = %config.bind_addr, config = %config_path.display(), "listening");

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

fn parse_config_path() -> anyhow::Result<Option<PathBuf>> {
    let mut args = env::args_os().skip(1);
    let mut config_path = None;

    while let Some(arg) = args.next() {
        let value = arg.to_string_lossy();
        match value.as_ref() {
            "-h" | "--help" => return Ok(None),
            "-c" | "--config" => {
                let path = args
                    .next()
                    .context("--config requires a config file path")?;
                config_path = Some(PathBuf::from(path));
            }
            other if other.starts_with("--config=") => {
                config_path = Some(PathBuf::from(other.trim_start_matches("--config=")));
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    Ok(Some(
        config_path
            .or_else(|| env::var_os("JM_BOT_CONFIG").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH)),
    ))
}

fn print_usage() {
    println!(
        "Usage: jmcomic-bot-service [--config <path>]\n\nDefault config path: {DEFAULT_CONFIG_PATH}\nEnvironment fallback: JM_BOT_CONFIG"
    );
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
