use std::{str::FromStr, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    middleware,
    response::Response,
    routing::{get, post},
};
use serde_json::json;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    auth::require_auth,
    config::Config,
    db::{ArtifactRecord, Db},
    error::{AppError, AppResult},
    image_processing::{get_slice_count, process_image},
    jobs::{
        JobQueue, album_photo_ids, fetch_album_cached, fetch_photo_cached,
        job_response_from_record, request_hash,
    },
    models::{
        AlbumInfo, ArtifactFormat, ArtifactResponse, CreateDownloadRequest, SearchQuery,
        SignedFileQuery, sorted_chapters,
    },
    signing::{signed_file_url, verify_artifact_signature},
    worker_client::WorkerClient,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    pub worker: WorkerClient,
    pub queue: JobQueue,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/search", get(search))
        .route("/albums/{album_id}", get(album_info))
        .route("/albums/{album_id}/cover", get(album_cover))
        .route("/downloads", post(create_download))
        .route("/downloads/{job_id}", get(download_status))
        .route("/artifacts/{artifact_id}", get(artifact_info))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .route("/health", get(health))
        .route("/api/v1/files/{artifact_id}", get(signed_file))
        .nest("/api/v1", protected)
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true }))
}

async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(state.worker.search(&query).await?))
}

async fn album_info(
    State(state): State<AppState>,
    Path(album_id): Path<String>,
) -> AppResult<Json<AlbumInfo>> {
    let album = fetch_album_cached(&state, &album_id).await?;
    Ok(Json(AlbumInfo {
        chapters: sorted_chapters(&album),
        album,
    }))
}

async fn album_cover(
    State(state): State<AppState>,
    Path(album_id): Path<String>,
) -> AppResult<Response> {
    let cover_dir = state.config.artifacts_dir().join("covers");
    tokio::fs::create_dir_all(&cover_dir).await?;
    let cover_path = cover_dir.join(format!("{album_id}.jpg"));

    if tokio::fs::try_exists(&cover_path).await? {
        return jpeg_file_response(cover_path, format!("{album_id}.jpg")).await;
    }

    let photo = fetch_photo_cached(&state, &album_id).await?;
    let first = photo
        .images
        .first()
        .ok_or_else(|| AppError::NotFound("cover image not found".to_owned()))?;
    let bytes = reqwest::Client::new()
        .get(&first.url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let slice_count = get_slice_count(photo.scramble_id, &photo.id, &first.name)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let quality = state.config.jpeg_quality;
    let processed =
        tokio::task::spawn_blocking(move || process_image(&bytes, slice_count, quality))
            .await
            .map_err(|error| AppError::Other(error.into()))?
            .map_err(AppError::Other)?;
    tokio::fs::write(&cover_path, processed.data).await?;

    jpeg_file_response(cover_path, format!("{album_id}.jpg")).await
}

async fn create_download(
    State(state): State<AppState>,
    Json(request): Json<CreateDownloadRequest>,
) -> AppResult<Json<crate::models::JobResponse>> {
    if request.album_id.trim().is_empty() {
        return Err(AppError::BadRequest("album_id is required".to_owned()));
    }

    let photo_ids = match request.photo_ids {
        Some(ids) if !ids.is_empty() => ids
            .into_iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>(),
        _ => {
            let album = fetch_album_cached(&state, &request.album_id).await?;
            album_photo_ids(&album)
        }
    };

    if photo_ids.is_empty() {
        return Err(AppError::BadRequest("no photo ids selected".to_owned()));
    }

    let request_hash = request_hash(&request.album_id, &photo_ids, request.format);

    if !request.force {
        if let Some(active) = state.db.find_active_job(&request_hash).await? {
            let download_url = active
                .artifact_id
                .as_ref()
                .map(|id| make_download_url(&state, id));
            return Ok(Json(job_response_from_record(active, false, download_url)));
        }

        if let Some(artifact) = state.db.find_cached_artifact(&request_hash).await? {
            if tokio::fs::try_exists(artifact.path_buf()).await? {
                let job_id = Uuid::new_v4().to_string();
                state
                    .db
                    .insert_job(
                        &job_id,
                        &request_hash,
                        request.format,
                        &request.album_id,
                        &serde_json::to_string(&photo_ids).expect("photo ids JSON"),
                        "completed",
                        "completed",
                        Some(&artifact.id),
                    )
                    .await?;
                let job = state.db.get_job(&job_id).await?.expect("inserted job");
                return Ok(Json(job_response_from_record(
                    job,
                    true,
                    Some(make_download_url(&state, &artifact.id)),
                )));
            }
        }
    }

    let job_id = Uuid::new_v4().to_string();
    state
        .db
        .insert_job(
            &job_id,
            &request_hash,
            request.format,
            &request.album_id,
            &serde_json::to_string(&photo_ids).expect("photo ids JSON"),
            "queued",
            "queued",
            None,
        )
        .await?;
    state.queue.enqueue(job_id.clone()).await?;
    let job = state.db.get_job(&job_id).await?.expect("inserted job");
    Ok(Json(job_response_from_record(job, false, None)))
}

async fn download_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> AppResult<Json<crate::models::JobResponse>> {
    let job = state
        .db
        .get_job(&job_id)
        .await?
        .ok_or_else(|| AppError::NotFound("job not found".to_owned()))?;
    let download_url = job
        .artifact_id
        .as_ref()
        .map(|artifact_id| make_download_url(&state, artifact_id));
    Ok(Json(job_response_from_record(job, false, download_url)))
}

async fn artifact_info(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
) -> AppResult<Json<ArtifactResponse>> {
    let artifact = state
        .db
        .get_artifact(&artifact_id)
        .await?
        .ok_or_else(|| AppError::NotFound("artifact not found".to_owned()))?;
    Ok(Json(artifact_response(&state, artifact)))
}

async fn signed_file(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    Query(query): Query<SignedFileQuery>,
) -> AppResult<Response> {
    if !verify_artifact_signature(
        &state.config.file_signing_secret,
        &artifact_id,
        query.exp,
        &query.sig,
    ) {
        return Err(AppError::Unauthorized);
    }

    let artifact = state
        .db
        .get_artifact(&artifact_id)
        .await?
        .ok_or_else(|| AppError::NotFound("artifact not found".to_owned()))?;
    if artifact.expires_at < crate::signing::now_unix() {
        return Err(AppError::NotFound("artifact expired".to_owned()));
    }

    let path = artifact.path_buf();
    if !tokio::fs::try_exists(&path).await? {
        return Err(AppError::NotFound("artifact file missing".to_owned()));
    }

    state.db.touch_artifact(&artifact_id).await?;
    let format = ArtifactFormat::from_str(&artifact.format).unwrap_or(ArtifactFormat::Zip);
    let filename = format!(
        "{}.{}",
        crate::packaging::sanitize_archive_segment(&artifact.title),
        format.extension()
    );
    stream_file_response(path, format.content_type(), filename).await
}

fn artifact_response(state: &AppState, artifact: ArtifactRecord) -> ArtifactResponse {
    ArtifactResponse {
        artifact_id: artifact.id.clone(),
        format: artifact.format,
        title: artifact.title,
        size_bytes: artifact.size_bytes,
        sha256: artifact.sha256,
        page_count: artifact.page_count,
        download_url: make_download_url(state, &artifact.id),
        created_at: artifact.created_at,
        last_accessed_at: artifact.last_accessed_at,
        expires_at: artifact.expires_at,
    }
}

fn make_download_url(state: &AppState, artifact_id: &str) -> String {
    let relative = signed_file_url(
        &state.config.file_signing_secret,
        artifact_id,
        state.config.signed_url_ttl_seconds,
    );
    if let Some(base) = &state.config.public_base_url {
        if let Ok(base) = reqwest::Url::parse(base) {
            if let Ok(url) = base.join(relative.trim_start_matches('/')) {
                return url.to_string();
            }
        }
    }
    relative
}

async fn jpeg_file_response(path: std::path::PathBuf, filename: String) -> AppResult<Response> {
    stream_file_response(path, "image/jpeg", filename).await
}

async fn stream_file_response(
    path: std::path::PathBuf,
    content_type: &str,
    filename: String,
) -> AppResult<Response> {
    let file = tokio::fs::File::open(path).await?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename.replace('"', "_")),
        )
        .body(body)
        .map_err(|error| AppError::Other(error.into()))
}
