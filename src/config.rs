use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::Deserialize;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/jmcomic-bot-service/config.json";

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
    pub fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let file = serde_json::from_str::<ConfigFile>(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;

        let bot_tokens = file
            .bot_tokens
            .into_iter()
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        if bot_tokens.is_empty() {
            bail!("config.bot_tokens must contain at least one token");
        }

        if file.file_signing_secret.trim().is_empty() {
            bail!("config.file_signing_secret must not be empty");
        }

        if file.worker_base_url.trim().is_empty() {
            bail!("config.worker_base_url must not be empty");
        }

        if !(1..=100).contains(&file.jpeg_quality) {
            bail!("config.jpeg_quality must be between 1 and 100");
        }
        if file.max_concurrent_jobs == 0 {
            bail!("config.max_concurrent_jobs must be greater than 0");
        }
        if file.image_concurrency == 0 {
            bail!("config.image_concurrency must be greater than 0");
        }
        if file.signed_url_ttl_seconds <= 0 {
            bail!("config.signed_url_ttl_seconds must be greater than 0");
        }
        if file.artifact_ttl_days <= 0 {
            bail!("config.artifact_ttl_days must be greater than 0");
        }
        if file.max_pages_per_job == 0 {
            bail!("config.max_pages_per_job must be greater than 0");
        }

        let database_url = file
            .database_url
            .unwrap_or_else(|| sqlite_url_for_data_dir(&file.data_dir));
        let bind_addr = file
            .bind_addr
            .parse::<SocketAddr>()
            .with_context(|| "config.bind_addr must be a socket address")?;

        Ok(Self {
            bot_tokens,
            file_signing_secret: file.file_signing_secret,
            worker_base_url: file.worker_base_url,
            data_dir: file.data_dir,
            database_url,
            max_concurrent_jobs: file.max_concurrent_jobs,
            image_concurrency: file.image_concurrency,
            signed_url_ttl_seconds: file.signed_url_ttl_seconds,
            artifact_ttl_days: file.artifact_ttl_days,
            cache_max_bytes: file.cache_max_bytes,
            max_pages_per_job: file.max_pages_per_job,
            bind_addr,
            jpeg_quality: file.jpeg_quality,
            public_base_url: file.public_base_url,
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

#[derive(Debug, Deserialize)]
struct ConfigFile {
    bot_tokens: Vec<String>,
    file_signing_secret: String,
    worker_base_url: String,
    #[serde(default = "default_data_dir")]
    data_dir: PathBuf,
    #[serde(default)]
    database_url: Option<String>,
    #[serde(default = "default_max_concurrent_jobs")]
    max_concurrent_jobs: usize,
    #[serde(default = "default_image_concurrency")]
    image_concurrency: usize,
    #[serde(default = "default_signed_url_ttl_seconds")]
    signed_url_ttl_seconds: i64,
    #[serde(default = "default_artifact_ttl_days")]
    artifact_ttl_days: i64,
    #[serde(default = "default_cache_max_bytes")]
    cache_max_bytes: u64,
    #[serde(default = "default_max_pages_per_job")]
    max_pages_per_job: usize,
    #[serde(default = "default_bind_addr")]
    bind_addr: String,
    #[serde(default = "default_jpeg_quality")]
    jpeg_quality: u8,
    #[serde(default)]
    public_base_url: Option<String>,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/jmcomic-bot-service")
}

fn default_max_concurrent_jobs() -> usize {
    2
}

fn default_image_concurrency() -> usize {
    6
}

fn default_signed_url_ttl_seconds() -> i64 {
    3600
}

fn default_artifact_ttl_days() -> i64 {
    30
}

fn default_cache_max_bytes() -> u64 {
    53_687_091_200
}

fn default_max_pages_per_job() -> usize {
    800
}

fn default_bind_addr() -> String {
    "0.0.0.0:3000".to_owned()
}

fn default_jpeg_quality() -> u8 {
    90
}

fn sqlite_url_for_data_dir(data_dir: &std::path::Path) -> String {
    format!("sqlite://{}", data_dir.join("jm-bot.db").display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
              "bot_tokens": ["dev"],
              "file_signing_secret": "dev-secret",
              "worker_base_url": "http://127.0.0.1:8787"
            }"#,
        )
        .unwrap();

        let config = Config::from_file(&path).unwrap();
        assert_eq!(config.bot_tokens, vec!["dev"]);
        assert_eq!(config.bind_addr.to_string(), "0.0.0.0:3000");
        assert_eq!(
            config.data_dir,
            PathBuf::from("/var/lib/jmcomic-bot-service")
        );
        assert_eq!(
            config.database_url,
            "sqlite:///var/lib/jmcomic-bot-service/jm-bot.db"
        );
    }

    #[test]
    fn config_file_rejects_empty_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
              "bot_tokens": ["", "  "],
              "file_signing_secret": "dev-secret",
              "worker_base_url": "http://127.0.0.1:8787"
            }"#,
        )
        .unwrap();

        let error = Config::from_file(&path).unwrap_err().to_string();
        assert!(error.contains("config.bot_tokens"));
    }

    #[test]
    fn config_file_rejects_invalid_numbers_and_addresses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
              "bot_tokens": ["dev"],
              "file_signing_secret": "dev-secret",
              "worker_base_url": "http://127.0.0.1:8787",
              "bind_addr": "not-an-address",
              "jpeg_quality": 0
            }"#,
        )
        .unwrap();

        let error = Config::from_file(&path).unwrap_err().to_string();
        assert!(error.contains("config.jpeg_quality"));
    }

    #[test]
    fn config_file_uses_explicit_database_url() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            format!(
                r#"{{
                  "bot_tokens": ["dev"],
                  "file_signing_secret": "dev-secret",
                  "worker_base_url": "http://127.0.0.1:8787",
                  "data_dir": "{}",
                  "database_url": "sqlite:///tmp/custom.db",
                  "bind_addr": "127.0.0.1:3001",
                  "jpeg_quality": 75
                }}"#,
                data_dir.display()
            ),
        )
        .unwrap();

        let config = Config::from_file(&path).unwrap();
        assert_eq!(config.data_dir, data_dir);
        assert_eq!(config.database_url, "sqlite:///tmp/custom.db");
        assert_eq!(config.bind_addr.to_string(), "127.0.0.1:3001");
        assert_eq!(config.jpeg_quality, 75);
    }
}
