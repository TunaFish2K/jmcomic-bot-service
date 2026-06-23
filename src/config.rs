use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, bail};

#[derive(Debug, Clone)]
pub struct Config {
    pub bot_tokens: Vec<String>,
    pub file_signing_secret: String,
    pub worker_base_url: String,
    pub data_dir: PathBuf,
    pub database_url: String,
    pub max_concurrent_jobs: usize,
    pub image_concurrency: usize,
    pub signed_url_ttl_seconds: i64,
    pub artifact_ttl_days: i64,
    pub cache_max_bytes: u64,
    pub max_pages_per_job: usize,
    pub bind_addr: SocketAddr,
    pub jpeg_quality: u8,
    pub public_base_url: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let data_dir = PathBuf::from(env_string("DATA_DIR", "/data"));
        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| sqlite_url_for_data_dir(&data_dir));

        let bot_tokens = env::var("BOT_TOKENS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if bot_tokens.is_empty() {
            bail!("BOT_TOKENS must contain at least one bearer token");
        }

        let file_signing_secret = env::var("FILE_SIGNING_SECRET")
            .context("FILE_SIGNING_SECRET must be set for signed file URLs")?;
        if file_signing_secret.len() < 16 {
            bail!("FILE_SIGNING_SECRET should be at least 16 characters");
        }

        let worker_base_url =
            env::var("JM_WORKER_BASE_URL").context("JM_WORKER_BASE_URL must be set")?;

        Ok(Self {
            bot_tokens,
            file_signing_secret,
            worker_base_url,
            data_dir,
            database_url,
            max_concurrent_jobs: env_parse("MAX_CONCURRENT_JOBS", 2)?,
            image_concurrency: env_parse("IMAGE_CONCURRENCY", 6)?,
            signed_url_ttl_seconds: env_parse("SIGNED_URL_TTL_SECONDS", 3600)?,
            artifact_ttl_days: env_parse("ARTIFACT_TTL_DAYS", 30)?,
            cache_max_bytes: env_parse("CACHE_MAX_BYTES", 53_687_091_200_u64)?,
            max_pages_per_job: env_parse("MAX_PAGES_PER_JOB", 800)?,
            bind_addr: env_string("BIND_ADDR", "0.0.0.0:3000")
                .parse()
                .context("BIND_ADDR must be a socket address")?,
            jpeg_quality: env_parse("JPEG_QUALITY", 90)?,
            public_base_url: env::var("PUBLIC_BASE_URL").ok(),
        })
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.data_dir.join("artifacts")
    }

    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }

    pub async fn ensure_dirs(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.data_dir).await?;
        tokio::fs::create_dir_all(self.artifacts_dir()).await?;
        tokio::fs::create_dir_all(self.tmp_dir()).await?;
        Ok(())
    }
}

fn env_string(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn env_parse<T>(key: &str, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(key) {
        Ok(value) => value
            .parse::<T>()
            .with_context(|| format!("{key} has invalid value")),
        Err(_) => Ok(default),
    }
}

fn sqlite_url_for_data_dir(data_dir: &std::path::Path) -> String {
    format!("sqlite://{}", data_dir.join("jm-bot.db").display())
}
