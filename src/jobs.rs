use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::Context;
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore, mpsc};
use uuid::Uuid;

use crate::{
    db::JobRecord,
    error::{AppError, AppResult},
    image_processing::{ProcessedImage, get_slice_count, process_image},
    models::{Album, ArtifactFormat, Photo, sorted_chapters},
    packaging::{ArchivePhoto, write_artifact},
    routes::AppState,
};

#[derive(Clone)]
pub struct JobQueue {
    tx: mpsc::Sender<String>,
    rx: Arc<Mutex<mpsc::Receiver<String>>>,
    workers: usize,
}

impl JobQueue {
    pub fn new(workers: usize) -> Self {
        let (tx, rx) = mpsc::channel(1024);
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
            workers: workers.max(1),
        }
    }

    pub async fn enqueue(&self, job_id: String) -> AppResult<()> {
        self.tx
            .send(job_id)
            .await
            .map_err(|_| AppError::Conflict("job queue is closed".to_owned()))
    }
}

pub fn spawn_workers(queue: JobQueue, state: AppState) {
    for worker_index in 0..queue.workers {
        let queue = queue.clone();
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                let job_id = {
                    let mut rx = queue.rx.lock().await;
                    rx.recv().await
                };

                let Some(job_id) = job_id else {
                    break;
                };

                tracing::info!(worker_index, %job_id, "starting job");
                if let Err(error) = process_job(state.clone(), job_id.clone()).await {
                    tracing::error!(worker_index, %job_id, error = %error, "job failed");
                    let _ = state.db.fail_job(&job_id, &error.to_string()).await;
                }
            }
        });
    }
}

pub async fn process_job(state: AppState, job_id: String) -> anyhow::Result<()> {
    let job = state
        .db
        .get_job(&job_id)
        .await?
        .context("job record disappeared")?;
    let format = ArtifactFormat::from_str(&job.format)
        .map_err(|message| anyhow::anyhow!("invalid job format: {message}"))?;
    let photo_ids: Vec<String> = serde_json::from_str(&job.photo_ids_json)?;

    state
        .db
        .update_job_progress(&job_id, "running", "metadata", 0, 0)
        .await?;

    let album = fetch_album_cached(&state, &job.album_id).await?;
    let photos = fetch_photos_cached(&state, &photo_ids).await?;
    let page_count = photos.iter().map(|photo| photo.images.len()).sum::<usize>();

    if page_count == 0 {
        anyhow::bail!("no images found for requested chapters");
    }
    if page_count > state.config.max_pages_per_job {
        anyhow::bail!(
            "job has {page_count} pages, exceeds MAX_PAGES_PER_JOB={}",
            state.config.max_pages_per_job
        );
    }

    state
        .db
        .update_job_progress(&job_id, "running", "downloading", 0, page_count as i64)
        .await?;

    let processed = download_and_process_photos(&state, &job_id, &photos, page_count).await?;

    state
        .db
        .update_job_progress(
            &job_id,
            "running",
            "archive",
            page_count as i64,
            page_count as i64,
        )
        .await?;

    let artifact_id = Uuid::new_v4().to_string();
    let artifact_path = state
        .config
        .artifacts_dir()
        .join(format!("{artifact_id}.{}", format.extension()));
    let packaging_photos = processed.clone();
    let format_for_task = format;
    let path_for_task = artifact_path.clone();
    tokio::task::spawn_blocking(move || {
        write_artifact(&path_for_task, format_for_task, &packaging_photos)
    })
    .await??;

    let metadata = tokio::fs::metadata(&artifact_path).await?;
    let digest = sha256_file(&artifact_path).await?;
    state
        .db
        .insert_artifact(
            &artifact_id,
            &job.request_hash,
            format,
            &album.name,
            &artifact_path.to_string_lossy(),
            metadata.len() as i64,
            &digest,
            page_count as i64,
            state.config.artifact_ttl_days,
        )
        .await?;
    state.db.complete_job(&job_id, &artifact_id).await?;

    tracing::info!(%job_id, %artifact_id, page_count, "job completed");
    Ok(())
}

async fn download_and_process_photos(
    state: &AppState,
    job_id: &str,
    photos: &[Photo],
    page_count: usize,
) -> anyhow::Result<Vec<ArchivePhoto>> {
    let http = reqwest::Client::builder()
        .user_agent("jmcomic-bot-service/0.1")
        .build()?;
    let semaphore = Arc::new(Semaphore::new(state.config.image_concurrency.max(1)));
    let mut tasks = FuturesUnordered::new();
    let mut flat_index = 0_usize;

    for (photo_index, photo) in photos.iter().enumerate() {
        for (image_index, image) in photo.images.iter().enumerate() {
            let page = PageTask {
                flat_index,
                photo_index,
                image_index,
                photo_id: photo.id.clone(),
                scramble_id: photo.scramble_id,
                filename: image.name.clone(),
                url: image.url.clone(),
            };
            flat_index += 1;

            let http = http.clone();
            let semaphore = semaphore.clone();
            let jpeg_quality = state.config.jpeg_quality;
            tasks.push(tokio::spawn(async move {
                let _permit = semaphore.acquire_owned().await?;
                process_page(http, page, jpeg_quality).await
            }));
        }
    }

    let mut buckets = photos
        .iter()
        .map(|photo| {
            let mut images = Vec::with_capacity(photo.images.len());
            images.resize_with(photo.images.len(), || None);
            (photo.name.clone(), images)
        })
        .collect::<Vec<_>>();
    let mut done = 0_i64;

    while let Some(result) = tasks.next().await {
        let page = result??;
        buckets[page.photo_index].1[page.image_index] = Some(page.image);
        done += 1;
        state
            .db
            .update_job_progress(job_id, "running", "downloading", done, page_count as i64)
            .await?;
    }

    buckets
        .into_iter()
        .map(|(name, images)| {
            let images = images
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .context("internal error: processed page missing")?;
            Ok(ArchivePhoto { name, images })
        })
        .collect()
}

async fn process_page(
    http: reqwest::Client,
    task: PageTask,
    jpeg_quality: u8,
) -> anyhow::Result<PageResult> {
    let bytes = fetch_image_with_retries(&http, &task.url).await?;
    let slice_count = get_slice_count(task.scramble_id, &task.photo_id, &task.filename)?;
    let image =
        tokio::task::spawn_blocking(move || process_image(&bytes, slice_count, jpeg_quality))
            .await??;

    Ok(PageResult {
        flat_index: task.flat_index,
        photo_index: task.photo_index,
        image_index: task.image_index,
        image,
    })
}

async fn fetch_image_with_retries(http: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let delays = [400_u64, 1_000, 2_000];
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=delays.len() {
        match http.get(url).send().await {
            Ok(response) if response.status().is_success() => {
                return Ok(response.bytes().await?.to_vec());
            }
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                last_error = Some(anyhow::anyhow!("HTTP {status} for {url}: {body}"));
            }
            Err(error) => {
                last_error = Some(error.into());
            }
        }

        if let Some(delay) = delays.get(attempt) {
            tokio::time::sleep(std::time::Duration::from_millis(*delay)).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("image request failed for {url}")))
}

pub async fn fetch_album_cached(state: &AppState, album_id: &str) -> AppResult<Album> {
    if let Some(payload) = state.db.get_album_cache(album_id).await? {
        if let Ok(album) = serde_json::from_str::<Album>(&payload) {
            return Ok(album);
        }
    }

    let album = state.worker.album(album_id).await?;
    state
        .db
        .set_album_cache(
            album_id,
            &serde_json::to_string(&album).expect("album JSON"),
        )
        .await?;
    Ok(album)
}

pub async fn fetch_photo_cached(state: &AppState, photo_id: &str) -> AppResult<Photo> {
    if let Some(payload) = state.db.get_photo_cache(photo_id).await? {
        if let Ok(photo) = serde_json::from_str::<Photo>(&payload) {
            return Ok(photo);
        }
    }

    let photo = state.worker.photo(photo_id).await?;
    state
        .db
        .set_photo_cache(
            photo_id,
            &serde_json::to_string(&photo).expect("photo JSON"),
        )
        .await?;
    Ok(photo)
}

pub async fn fetch_photos_cached(state: &AppState, photo_ids: &[String]) -> AppResult<Vec<Photo>> {
    let mut found = HashMap::new();
    let mut missing = Vec::new();

    for photo_id in photo_ids {
        if let Some(payload) = state.db.get_photo_cache(photo_id).await? {
            if let Ok(photo) = serde_json::from_str::<Photo>(&payload) {
                found.insert(photo_id.clone(), photo);
                continue;
            }
        }
        missing.push(photo_id.clone());
    }

    if !missing.is_empty() {
        let fetched = state.worker.batch_photos(&missing).await?;
        for photo in fetched {
            state
                .db
                .set_photo_cache(
                    &photo.id,
                    &serde_json::to_string(&photo).expect("photo JSON"),
                )
                .await?;
            found.insert(photo.id.clone(), photo);
        }
    }

    photo_ids
        .iter()
        .map(|photo_id| {
            found.remove(photo_id).ok_or_else(|| {
                AppError::Upstream(format!("worker did not return photo {photo_id}"))
            })
        })
        .collect()
}

pub fn album_photo_ids(album: &Album) -> Vec<String> {
    sorted_chapters(album)
        .into_iter()
        .map(|chapter| chapter.id)
        .collect()
}

pub fn request_hash(album_id: &str, photo_ids: &[String], format: ArtifactFormat) -> String {
    #[derive(Serialize)]
    struct RequestHash<'a> {
        album_id: &'a str,
        photo_ids: &'a [String],
        format: String,
    }

    let payload = serde_json::to_vec(&RequestHash {
        album_id,
        photo_ids,
        format: format.to_string(),
    })
    .expect("request hash JSON");
    hex::encode(Sha256::digest(payload))
}

pub fn job_response_from_record(
    job: JobRecord,
    cached: bool,
    download_url: Option<String>,
) -> crate::models::JobResponse {
    crate::models::JobResponse {
        job_id: job.id,
        status: job.status,
        format: job.format,
        album_id: job.album_id,
        photo_ids: serde_json::from_str(&job.photo_ids_json).unwrap_or_default(),
        stage: job.stage,
        progress_done: job.progress_done,
        progress_total: job.progress_total,
        cached,
        artifact_id: job.artifact_id,
        download_url,
        error: job.error,
        created_at: job.created_at,
        updated_at: job.updated_at,
    }
}

async fn sha256_file(path: &PathBuf) -> anyhow::Result<String> {
    let bytes = tokio::fs::read(path).await?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[derive(Debug)]
struct PageTask {
    flat_index: usize,
    photo_index: usize,
    image_index: usize,
    photo_id: String,
    scramble_id: u32,
    filename: String,
    url: String,
}

#[derive(Debug)]
struct PageResult {
    #[allow(dead_code)]
    flat_index: usize,
    photo_index: usize,
    image_index: usize,
    image: ProcessedImage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_hash_is_stable_for_same_order() {
        let ids = vec!["2".to_owned(), "1".to_owned()];
        assert_eq!(
            request_hash("a", &ids, ArtifactFormat::Cbz),
            request_hash("a", &ids, ArtifactFormat::Cbz)
        );
    }

    #[test]
    fn request_hash_preserves_chapter_order() {
        let left = vec!["1".to_owned(), "2".to_owned()];
        let right = vec!["2".to_owned(), "1".to_owned()];
        assert_ne!(
            request_hash("a", &left, ArtifactFormat::Cbz),
            request_hash("a", &right, ArtifactFormat::Cbz)
        );
    }
}
