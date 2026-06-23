use std::{sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use image::{ImageEncoder, RgbImage, codecs::jpeg::JpegEncoder};
use jmcomic_bot_service::{
    config::Config,
    db::Db,
    jobs::{JobQueue, spawn_workers},
    models::JobResponse,
    routes::{AppState, router},
    worker_client::WorkerClient,
};
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn creates_cbz_artifact_from_mock_worker_and_cdn() {
    let upstream = MockServer::start().await;
    let image_bytes = tiny_jpeg();

    Mock::given(method("GET"))
        .and(path("/album/123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "123",
            "name": "Mock Album",
            "images": ["00001.jpg"],
            "description": "desc",
            "totalViews": "10",
            "likes": "2",
            "series": [],
            "seriesID": "",
            "author": ["author"],
            "tags": ["tag"],
            "works": [],
            "actors": []
        })))
        .mount(&upstream)
        .await;

    Mock::given(method("GET"))
        .and(path("/batch-photo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "photoId": "123",
                "photo": {
                    "id": "123",
                    "name": "Chapter 1",
                    "images": [
                        {
                            "name": "00001.jpg",
                            "url": format!("{}/media/photos/123/00001.jpg", upstream.uri())
                        }
                    ],
                    "scrambleId": 999999
                }
            }
        ])))
        .mount(&upstream)
        .await;

    Mock::given(method("GET"))
        .and(path("/media/photos/123/00001.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(image_bytes, "image/jpeg"))
        .mount(&upstream)
        .await;

    let temp = TempDir::new().unwrap();
    let state = test_state(&upstream, &temp).await;
    spawn_workers(state.queue.clone(), state.clone());
    let app = router(state.clone());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/downloads")
                .header(header::AUTHORIZATION, "Bearer test-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "album_id": "123",
                        "format": "cbz"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let created: JobResponse = serde_json::from_slice(&body).unwrap();

    let completed = wait_for_completed_job(&state.db, &created.job_id).await;
    assert_eq!(completed.status, "completed");
    let artifact_id = completed.artifact_id.expect("artifact id");
    let artifact = state
        .db
        .get_artifact(&artifact_id)
        .await
        .unwrap()
        .expect("artifact row");
    assert_eq!(artifact.format, "cbz");
    assert_eq!(artifact.page_count, 1);

    let bytes = tokio::fs::read(artifact.path).await.unwrap();
    assert!(bytes.starts_with(b"PK"));
}

async fn test_state(upstream: &MockServer, temp: &TempDir) -> AppState {
    let data_dir = temp.path().to_path_buf();
    let config = Arc::new(Config {
        bot_tokens: vec!["test-token".to_owned()],
        file_signing_secret: "test-signing-secret".to_owned(),
        worker_base_url: upstream.uri(),
        data_dir: data_dir.clone(),
        database_url: format!("sqlite://{}", data_dir.join("test.db").display()),
        max_concurrent_jobs: 1,
        image_concurrency: 2,
        signed_url_ttl_seconds: 60,
        artifact_ttl_days: 1,
        cache_max_bytes: 1_000_000_000,
        max_pages_per_job: 10,
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

async fn wait_for_completed_job(db: &Db, job_id: &str) -> jmcomic_bot_service::db::JobRecord {
    for _ in 0..100 {
        let job = db.get_job(job_id).await.unwrap().expect("job row");
        if job.status == "completed" {
            return job;
        }
        if job.status == "failed" {
            panic!("job failed: {:?}", job.error);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("job did not complete in time");
}

fn tiny_jpeg() -> Vec<u8> {
    let mut image = RgbImage::new(2, 2);
    for y in 0..2 {
        for x in 0..2 {
            image.put_pixel(x, y, image::Rgb([x as u8 * 100, y as u8 * 100, 32]));
        }
    }
    let mut bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut bytes, 90);
    encoder
        .write_image(image.as_raw(), 2, 2, image::ExtendedColorType::Rgb8)
        .unwrap();
    bytes
}
