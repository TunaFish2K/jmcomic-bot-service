use std::{path::PathBuf, str::FromStr};

use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::{models::ArtifactFormat, signing::now_unix};

#[derive(Debug, Clone)]
pub struct Db {
    pool: SqlitePool,
}

#[derive(Debug, Clone)]
pub struct JobRecord {
    pub id: String,
    pub request_hash: String,
    pub status: String,
    pub format: String,
    pub album_id: String,
    pub photo_ids_json: String,
    pub stage: String,
    pub progress_done: i64,
    pub progress_total: i64,
    pub artifact_id: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct ArtifactRecord {
    pub id: String,
    pub request_hash: String,
    pub format: String,
    pub title: String,
    pub path: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub page_count: i64,
    pub created_at: i64,
    pub last_accessed_at: i64,
    pub expires_at: i64,
}

impl Db {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)?.create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    pub async fn init(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                request_hash TEXT NOT NULL,
                status TEXT NOT NULL,
                format TEXT NOT NULL,
                album_id TEXT NOT NULL,
                photo_ids_json TEXT NOT NULL,
                stage TEXT NOT NULL,
                progress_done INTEGER NOT NULL DEFAULT 0,
                progress_total INTEGER NOT NULL DEFAULT 0,
                artifact_id TEXT,
                error TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_jobs_request_hash_status
            ON jobs(request_hash, status, updated_at);
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                request_hash TEXT NOT NULL,
                format TEXT NOT NULL,
                title TEXT NOT NULL,
                path TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                sha256 TEXT NOT NULL,
                page_count INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                last_accessed_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_artifacts_request_hash
            ON artifacts(request_hash, created_at DESC);
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS album_cache (
                album_id TEXT PRIMARY KEY,
                payload_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS photo_cache (
                photo_id TEXT PRIMARY KEY,
                payload_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_album_cache(&self, album_id: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT payload_json FROM album_cache WHERE album_id = ?")
            .bind(album_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| row.get("payload_json")))
    }

    pub async fn set_album_cache(&self, album_id: &str, payload_json: &str) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO album_cache(album_id, payload_json, updated_at)
            VALUES(?, ?, ?)
            ON CONFLICT(album_id) DO UPDATE SET
                payload_json = excluded.payload_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(album_id)
        .bind(payload_json)
        .bind(now_unix())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_photo_cache(&self, photo_id: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT payload_json FROM photo_cache WHERE photo_id = ?")
            .bind(photo_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| row.get("payload_json")))
    }

    pub async fn set_photo_cache(&self, photo_id: &str, payload_json: &str) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO photo_cache(photo_id, payload_json, updated_at)
            VALUES(?, ?, ?)
            ON CONFLICT(photo_id) DO UPDATE SET
                payload_json = excluded.payload_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(photo_id)
        .bind(payload_json)
        .bind(now_unix())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_job(
        &self,
        id: &str,
        request_hash: &str,
        format: ArtifactFormat,
        album_id: &str,
        photo_ids_json: &str,
        status: &str,
        stage: &str,
        artifact_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = now_unix();
        sqlx::query(
            r#"
            INSERT INTO jobs(
                id, request_hash, status, format, album_id, photo_ids_json, stage,
                progress_done, progress_total, artifact_id, error, created_at, updated_at
            )
            VALUES(?, ?, ?, ?, ?, ?, ?, 0, 0, ?, NULL, ?, ?)
            "#,
        )
        .bind(id)
        .bind(request_hash)
        .bind(status)
        .bind(format.to_string())
        .bind(album_id)
        .bind(photo_ids_json)
        .bind(stage)
        .bind(artifact_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_job_progress(
        &self,
        id: &str,
        status: &str,
        stage: &str,
        done: i64,
        total: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET status = ?, stage = ?, progress_done = ?, progress_total = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status)
        .bind(stage)
        .bind(done)
        .bind(total)
        .bind(now_unix())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn complete_job(&self, id: &str, artifact_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'completed', stage = 'completed', artifact_id = ?, error = NULL, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(artifact_id)
        .bind(now_unix())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn fail_job(&self, id: &str, message: &str) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'failed', stage = 'failed', error = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(message)
        .bind(now_unix())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_job(&self, id: &str) -> anyhow::Result<Option<JobRecord>> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(job_from_row))
    }

    pub async fn find_active_job(&self, request_hash: &str) -> anyhow::Result<Option<JobRecord>> {
        let row = sqlx::query(
            r#"
            SELECT * FROM jobs
            WHERE request_hash = ? AND status IN ('queued', 'running')
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(request_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(job_from_row))
    }

    pub async fn find_cached_artifact(
        &self,
        request_hash: &str,
    ) -> anyhow::Result<Option<ArtifactRecord>> {
        let now = now_unix();
        let row = sqlx::query(
            r#"
            SELECT * FROM artifacts
            WHERE request_hash = ? AND expires_at > ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(request_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(artifact_from_row))
    }

    pub async fn insert_artifact(
        &self,
        id: &str,
        request_hash: &str,
        format: ArtifactFormat,
        title: &str,
        path: &str,
        size_bytes: i64,
        sha256: &str,
        page_count: i64,
        ttl_days: i64,
    ) -> anyhow::Result<()> {
        let now = now_unix();
        sqlx::query(
            r#"
            INSERT INTO artifacts(
                id, request_hash, format, title, path, size_bytes, sha256, page_count,
                created_at, last_accessed_at, expires_at
            )
            VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(request_hash)
        .bind(format.to_string())
        .bind(title)
        .bind(path)
        .bind(size_bytes)
        .bind(sha256)
        .bind(page_count)
        .bind(now)
        .bind(now)
        .bind(now + ttl_days * 86_400)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_artifact(&self, id: &str) -> anyhow::Result<Option<ArtifactRecord>> {
        let row = sqlx::query("SELECT * FROM artifacts WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(artifact_from_row))
    }

    pub async fn touch_artifact(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE artifacts SET last_accessed_at = ? WHERE id = ?")
            .bind(now_unix())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl ArtifactRecord {
    pub fn path_buf(&self) -> PathBuf {
        PathBuf::from(&self.path)
    }
}

fn job_from_row(row: sqlx::sqlite::SqliteRow) -> JobRecord {
    JobRecord {
        id: row.get("id"),
        request_hash: row.get("request_hash"),
        status: row.get("status"),
        format: row.get("format"),
        album_id: row.get("album_id"),
        photo_ids_json: row.get("photo_ids_json"),
        stage: row.get("stage"),
        progress_done: row.get("progress_done"),
        progress_total: row.get("progress_total"),
        artifact_id: row.get("artifact_id"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn artifact_from_row(row: sqlx::sqlite::SqliteRow) -> ArtifactRecord {
    ArtifactRecord {
        id: row.get("id"),
        request_hash: row.get("request_hash"),
        format: row.get("format"),
        title: row.get("title"),
        path: row.get("path"),
        size_bytes: row.get("size_bytes"),
        sha256: row.get("sha256"),
        page_count: row.get("page_count"),
        created_at: row.get("created_at"),
        last_accessed_at: row.get("last_accessed_at"),
        expires_at: row.get("expires_at"),
    }
}
