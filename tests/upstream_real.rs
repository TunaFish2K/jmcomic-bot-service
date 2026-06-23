use std::{env, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use jmcomic_bot_service::{
    config::Config,
    db::Db,
    jobs::{JobQueue, spawn_workers},
    models::{ArtifactResponse, JobResponse},
    routes::{AppState, router},
    worker_client::WorkerClient,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires VPN/network access to a real JM Worker and CDN"]
async fn real_worker_cover_cbz_and_pdf_download() {
    let worker_base_url = env::var("JM_REAL_WORKER_BASE_URL")
        .expect("JM_REAL_WORKER_BASE_URL is required for this ignored test");
    let album_id =
        env::var("JM_REAL_ALBUM_ID").expect("JM_REAL_ALBUM_ID is required for this ignored test");
    let token = env::var("JM_REAL_BOT_TOKEN").unwrap_or_else(|_| "real-test-token".to_owned());

    let temp = TempDir::new().unwrap();
    let state = real_state(worker_base_url, token.clone(), &temp).await;
    spawn_workers(state.queue.clone(), state.clone());
    let app = router(state);

    let cover = request(
        app.clone(),
        Method::GET,
        &format!("/api/v1/albums/{album_id}/cover"),
        None,
        Some(&token),
    )
    .await;
    assert_eq!(cover.status(), StatusCode::OK);
    let cover_body = to_bytes(cover.into_body(), 50 * 1024 * 1024).await.unwrap();
    assert!(cover_body.starts_with(&[0xff, 0xd8]));

    let cbz = run_real_download(app.clone(), &token, &album_id, "cbz").await;
    assert_eq!(cbz.format, "cbz");
    let cbz_response = request(app.clone(), Method::GET, &cbz.download_url, None, None).await;
    assert_eq!(cbz_response.status(), StatusCode::OK);
    let cbz_body = to_bytes(cbz_response.into_body(), 250 * 1024 * 1024)
        .await
        .unwrap();
    assert!(cbz_body.starts_with(b"PK"));

    let pdf = run_real_download(app.clone(), &token, &album_id, "pdf").await;
    assert_eq!(pdf.format, "pdf");
    let pdf_response = request(app, Method::GET, &pdf.download_url, None, None).await;
    assert_eq!(pdf_response.status(), StatusCode::OK);
    let pdf_body = to_bytes(pdf_response.into_body(), 250 * 1024 * 1024)
        .await
        .unwrap();
    assert!(pdf_body.starts_with(b"%PDF-"));
    assert!(String::from_utf8_lossy(&pdf_body).contains("%%EOF"));
}

async fn run_real_download(
    app: Router,
    token: &str,
    album_id: &str,
    format: &str,
) -> ArtifactResponse {
    let created: JobResponse = json_request(
        app.clone(),
        Method::POST,
        "/api/v1/downloads",
        Some(json!({
            "album_id": album_id,
            "photo_ids": [album_id],
            "format": format,
            "force": true
        })),
        Some(token),
    )
    .await;

    let completed = wait_for_completed_job(app.clone(), token, &created.job_id).await;
    let artifact_id = completed.artifact_id.expect("completed job artifact id");
    json_request(
        app,
        Method::GET,
        &format!("/api/v1/artifacts/{artifact_id}"),
        None,
        Some(token),
    )
    .await
}

async fn real_state(worker_base_url: String, token: String, temp: &TempDir) -> AppState {
    let data_dir = temp.path().join("data");
    let config = Arc::new(Config {
        bot_tokens: vec![token],
        file_signing_secret: "real-test-signing-secret".to_owned(),
        worker_base_url,
        data_dir: data_dir.clone(),
        database_url: format!("sqlite://{}", data_dir.join("test.db").display()),
        max_concurrent_jobs: 1,
        image_concurrency: env::var("JM_REAL_IMAGE_CONCURRENCY")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(4),
        signed_url_ttl_seconds: 600,
        artifact_ttl_days: 1,
        cache_max_bytes: 1_000_000_000,
        max_pages_per_job: env::var("JM_REAL_MAX_PAGES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1000),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        jpeg_quality: 90,
        public_base_url: None,
    });
    config.ensure_dirs().await.unwrap();
    let db = Db::connect(&config.database_url).await.unwrap();
    db.init().await.unwrap();

    AppState {
        config: config.clone(),
        db,
        worker: WorkerClient::new(config.worker_base_url.clone()).unwrap(),
        queue: JobQueue::new(1),
    }
}

async fn wait_for_completed_job(app: Router, token: &str, job_id: &str) -> JobResponse {
    for _ in 0..240 {
        let job: JobResponse = json_request(
            app.clone(),
            Method::GET,
            &format!("/api/v1/downloads/{job_id}"),
            None,
            Some(token),
        )
        .await;
        match job.status.as_str() {
            "completed" => return job,
            "failed" => panic!("real upstream job failed: {:?}", job.error),
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
    panic!("real upstream job did not complete in time");
}

async fn request(
    app: Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    app.oneshot(builder.body(body).unwrap()).await.unwrap()
}

async fn json_request<T>(
    app: Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> T
where
    T: serde::de::DeserializeOwned,
{
    let response = request(app, method, uri, body, token).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}
